//! Minimal internal percent-encoding support, matching the behavior of the
//! `percent-encoding` crate used by the reference `url` crate. Not exposed
//! as public API — `url` itself doesn't re-export `percent_encoding` either.

use std::borrow::Cow;
use std::str;

/// A set of ASCII bytes to percent-encode, built up with [`AsciiSet::add`].
///
/// <https://url.spec.whatwg.org/#percent-encoded-bytes>
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct AsciiSet {
    mask: [u32; 4],
}

const BITS_PER_CHUNK: usize = 32;

impl AsciiSet {
    const fn contains(&self, byte: u8) -> bool {
        let chunk = self.mask[byte as usize / BITS_PER_CHUNK];
        let bit = 1 << (byte as usize % BITS_PER_CHUNK);
        (chunk & bit) != 0
    }

    fn should_percent_encode(&self, byte: u8) -> bool {
        !byte.is_ascii() || self.contains(byte)
    }

    pub(crate) const fn add(&self, byte: u8) -> Self {
        let mut mask = self.mask;
        mask[byte as usize / BITS_PER_CHUNK] |= 1 << (byte as usize % BITS_PER_CHUNK);
        Self { mask }
    }
}

/// The C0 controls (0x00-0x1F) and DEL (0x7F).
///
/// <https://url.spec.whatwg.org/#c0-control-percent-encode-set>
pub(crate) const CONTROLS: &AsciiSet = &AsciiSet {
    mask: [!0_u32, 0, 0, 1 << (0x7F_u32 % 32)],
};

/// The unconditional `%XX` percent-encoding of a single byte.
pub(crate) fn percent_encode_byte(byte: u8) -> &'static str {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    static TABLE: [[u8; 3]; 256] = {
        let mut table = [[0u8; 3]; 256];
        let mut b = 0usize;
        while b < 256 {
            table[b] = [b'%', HEX[b >> 4], HEX[b & 0xF]];
            b += 1;
        }
        table
    };
    // SAFETY: every entry is `%` followed by two ASCII hex digits.
    unsafe { str::from_utf8_unchecked(&TABLE[byte as usize]) }
}

/// Percent-encode the UTF-8 encoding of `input`, escaping bytes in `ascii_set`
/// (and all non-ASCII bytes).
pub(crate) fn utf8_percent_encode<'a>(
    input: &'a str,
    ascii_set: &'static AsciiSet,
) -> Cow<'a, str> {
    if !input.bytes().any(|b| ascii_set.should_percent_encode(b)) {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if ascii_set.should_percent_encode(byte) {
            out.push_str(percent_encode_byte(byte));
        } else {
            out.push(byte as char);
        }
    }
    Cow::Owned(out)
}

/// Percent-encode raw bytes, escaping bytes in `ascii_set` (and all
/// non-ASCII bytes). Unlike [`utf8_percent_encode`], `input` need not be
/// valid UTF-8 (used for `query_encoding_override` results).
pub(crate) fn percent_encode<'a>(input: &'a [u8], ascii_set: &'static AsciiSet) -> Cow<'a, [u8]> {
    if !input.iter().any(|&b| ascii_set.should_percent_encode(b)) {
        return Cow::Borrowed(input);
    }
    let mut out = Vec::with_capacity(input.len());
    for &byte in input {
        if ascii_set.should_percent_encode(byte) {
            out.extend_from_slice(percent_encode_byte(byte).as_bytes());
        } else {
            out.push(byte);
        }
    }
    Cow::Owned(out)
}

fn after_percent_sign(bytes: &[u8], i: &mut usize) -> Option<u8> {
    let h = (*bytes.get(*i)? as char).to_digit(16)?;
    let l = (*bytes.get(*i + 1)? as char).to_digit(16)?;
    *i += 2;
    Some((h as u8) * 0x10 + l as u8)
}

/// Percent-decode `input`. <https://url.spec.whatwg.org/#string-percent-decode>
pub(crate) fn percent_decode(input: &[u8]) -> Cow<'_, [u8]> {
    if !input.contains(&b'%') {
        return Cow::Borrowed(input);
    }
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let byte = input[i];
        i += 1;
        if byte == b'%' {
            if let Some(decoded) = after_percent_sign(input, &mut i) {
                out.push(decoded);
                continue;
            }
        }
        out.push(byte);
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`');

    #[test]
    fn encodes_controls_and_set_members() {
        assert_eq!(
            utf8_percent_encode("foo <bar>", FRAGMENT),
            "foo%20%3Cbar%3E"
        );
        assert_eq!(utf8_percent_encode("foo", FRAGMENT), Cow::Borrowed("foo"));
    }

    #[test]
    fn encodes_non_ascii() {
        assert_eq!(utf8_percent_encode("é", CONTROLS), "%C3%A9");
    }

    #[test]
    fn decode_round_trips() {
        assert_eq!(&*percent_decode(b"foo%20%3Cbar%3E"), b"foo <bar>");
        assert_eq!(
            &*percent_decode(b"no percent here"),
            b"no percent here" as &[u8]
        );
    }

    #[test]
    fn decode_leaves_invalid_escapes_alone() {
        assert_eq!(&*percent_decode(b"100%"), b"100%" as &[u8]);
        assert_eq!(&*percent_decode(b"100%zz"), b"100%zz" as &[u8]);
    }
}
