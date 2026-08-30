//! HPACK integer and string literal primitives (RFC 7541 §5.1, §5.2).

use crate::error::{ErrorCode, H2Error, Result};
use crate::hpack::huffman;

/// Guard against the "integer overflow via unbounded continuation bytes"
/// attack noted in RFC 7541 §7: cap how many continuation octets we'll read.
const MAX_CONTINUATION_BYTES: usize = 10;

/// Encode `value` using an N-bit prefix (RFC 7541 §5.1). `prefix_pattern`
/// is the flag bits for this representation already shifted into the top
/// `8 - prefix_bits` bits, with the low `prefix_bits` bits zeroed; the
/// function fills in those low bits (and any continuation bytes needed).
pub fn encode_integer(out: &mut Vec<u8>, prefix_bits: u8, prefix_pattern: u8, value: u64) {
    debug_assert!((1..=8).contains(&prefix_bits));
    let max_prefix = (1u64 << prefix_bits) - 1;
    if value < max_prefix {
        out.push(prefix_pattern | value as u8);
        return;
    }
    out.push(prefix_pattern | max_prefix as u8);
    let mut remaining = value - max_prefix;
    while remaining >= 128 {
        out.push(((remaining % 128) as u8) | 0x80);
        remaining /= 128;
    }
    out.push(remaining as u8);
}

/// Decode an N-bit-prefixed integer. Returns `(value, bytes_consumed)`.
pub fn decode_integer(buf: &[u8], prefix_bits: u8) -> Result<(u64, usize)> {
    debug_assert!((1..=8).contains(&prefix_bits));
    if buf.is_empty() {
        return Err(H2Error::Incomplete);
    }
    let max_prefix = (1u64 << prefix_bits) - 1;
    let prefix = buf[0] as u64 & max_prefix;
    if prefix < max_prefix {
        return Ok((prefix, 1));
    }

    let mut value = max_prefix;
    let mut m: u32 = 0;
    let mut consumed = 1;
    loop {
        if consumed > MAX_CONTINUATION_BYTES {
            return Err(H2Error::Connection(
                ErrorCode::CompressionError,
                "HPACK integer continuation too long",
            ));
        }
        if consumed >= buf.len() {
            return Err(H2Error::Incomplete);
        }
        let b = buf[consumed];
        consumed += 1;
        value = value
            .checked_add(((b & 0x7f) as u64) << m)
            .ok_or(H2Error::Connection(
                ErrorCode::CompressionError,
                "HPACK integer overflow",
            ))?;
        if b & 0x80 == 0 {
            break;
        }
        m += 7;
    }
    Ok((value, consumed))
}

/// Encode a string literal, automatically choosing Huffman coding when it
/// is strictly smaller than the raw bytes (RFC 7541 §5.2).
pub fn encode_string(out: &mut Vec<u8>, s: &[u8]) {
    let huff_bits = huffman::encoded_len_bits(s);
    let huff_bytes = huff_bits.div_ceil(8);
    if huff_bytes < s.len() {
        let encoded = huffman::encode(s);
        encode_integer(out, 7, 0x80, encoded.len() as u64);
        out.extend_from_slice(&encoded);
    } else {
        encode_integer(out, 7, 0x00, s.len() as u64);
        out.extend_from_slice(s);
    }
}

/// Decode a string literal. Returns `(bytes, total_consumed)`.
pub fn decode_string(buf: &[u8]) -> Result<(Vec<u8>, usize)> {
    if buf.is_empty() {
        return Err(H2Error::Incomplete);
    }
    let is_huffman = buf[0] & 0x80 != 0;
    let (len, len_bytes) = decode_integer(buf, 7)?;
    let len = len as usize;
    let end = len_bytes.checked_add(len).ok_or(H2Error::Connection(
        ErrorCode::CompressionError,
        "HPACK string length overflow",
    ))?;
    if buf.len() < end {
        return Err(H2Error::Incomplete);
    }
    let raw = &buf[len_bytes..end];
    let data = if is_huffman {
        huffman::decode(raw)?
    } else {
        raw.to_vec()
    };
    Ok((data, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7541_c1_1_small_value_fits_in_prefix() {
        // C.1.1: encoding 10 with a 5-bit prefix -> 0x0a
        let mut out = Vec::new();
        encode_integer(&mut out, 5, 0x00, 10);
        assert_eq!(out, vec![0x0a]);
        assert_eq!(decode_integer(&out, 5).unwrap(), (10, 1));
    }

    #[test]
    fn rfc7541_c1_2_value_needs_continuation() {
        // C.1.2: encoding 1337 with a 5-bit prefix -> 0x1f 0x9a 0x0a
        let mut out = Vec::new();
        encode_integer(&mut out, 5, 0x00, 1337);
        assert_eq!(out, vec![0x1f, 0x9a, 0x0a]);
        assert_eq!(decode_integer(&out, 5).unwrap(), (1337, 3));
    }

    #[test]
    fn rfc7541_c1_3_zero_prefix() {
        // C.1.3: encoding 42 with an 8-bit prefix -> 0x2a
        let mut out = Vec::new();
        encode_integer(&mut out, 8, 0x00, 42);
        assert_eq!(out, vec![0x2a]);
        assert_eq!(decode_integer(&out, 8).unwrap(), (42, 1));
    }

    #[test]
    fn string_roundtrip_huffman_and_raw() {
        for s in [b"".as_slice(), b"a", b"www.example.com", b"\x01\x02\x03"] {
            let mut out = Vec::new();
            encode_string(&mut out, s);
            let (decoded, consumed) = decode_string(&out).unwrap();
            assert_eq!(decoded, s);
            assert_eq!(consumed, out.len());
        }
    }

    #[test]
    fn overflow_continuation_rejected() {
        let buf = [
            0xffu8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01,
        ];
        assert!(decode_integer(&buf, 8).is_err());
    }

    #[test]
    fn incomplete_integer() {
        let buf = [0xffu8];
        assert_eq!(decode_integer(&buf, 8), Err(H2Error::Incomplete));
    }
}
