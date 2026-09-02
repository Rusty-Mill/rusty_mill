//! From-scratch Avro binary primitives -- no Avro crate exists anywhere
//! in this workspace (see [`crate::BaseEvent`]'s module doc), so this
//! is a minimal implementation of exactly the primitives the
//! `rusty_meshed` event family needs: the zigzag-varint `long`
//! (Avro's `int`/`long` share one variable-length encoding), `string`,
//! `array<string>`, and `double`. [`crate::BaseEvent`] and every
//! `rusty-meshed-domains` event build their own `serialize`/
//! `deserialize` on top of these rather than duplicating the
//! byte-level encoding rules themselves.

use rusty_err::Error;

/// Errors from decoding Avro binary built on these primitives.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AvroDecodeError {
    #[error("unexpected end of input decoding an Avro long")]
    UnexpectedEofInLong,
    #[error("unexpected end of input decoding an Avro string/bytes payload")]
    UnexpectedEofInPayload,
    #[error("Avro string/bytes length must be non-negative, got {0}")]
    NegativeLength(i64),
    #[error("invalid UTF-8 in an Avro string field")]
    InvalidUtf8,
    #[error("unexpected end of input decoding an Avro double")]
    UnexpectedEofInDouble,
}

/// Encodes an Avro `long`: zigzag then variable-length base-128,
/// least-significant group first, high bit set on every group but the
/// last.
pub fn encode_long(value: i64, out: &mut Vec<u8>) {
    let mut n = ((value << 1) ^ (value >> 63)) as u64;
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

/// Decodes an Avro `long` starting at `*pos`, advancing it past the
/// bytes consumed.
pub fn decode_long(bytes: &[u8], pos: &mut usize) -> Result<i64, AvroDecodeError> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *bytes
            .get(*pos)
            .ok_or(AvroDecodeError::UnexpectedEofInLong)?;
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Ok(((result >> 1) as i64) ^ -((result & 1) as i64))
}

/// Encodes an Avro `string`: a `long` byte-length prefix followed by
/// the UTF-8 bytes.
pub fn encode_string(s: &str, out: &mut Vec<u8>) {
    encode_long(s.len() as i64, out);
    out.extend_from_slice(s.as_bytes());
}

/// Decodes an Avro `string` starting at `*pos`, advancing it past the
/// bytes consumed.
pub fn decode_string(bytes: &[u8], pos: &mut usize) -> Result<String, AvroDecodeError> {
    let len = decode_long(bytes, pos)?;
    if len < 0 {
        return Err(AvroDecodeError::NegativeLength(len));
    }
    let end = *pos + len as usize;
    let slice = bytes
        .get(*pos..end)
        .ok_or(AvroDecodeError::UnexpectedEofInPayload)?;
    let s = std::str::from_utf8(slice)
        .map_err(|_| AvroDecodeError::InvalidUtf8)?
        .to_string();
    *pos = end;
    Ok(s)
}

/// Encodes an Avro `array<string>` -- one block (a positive item-count
/// `long`, then that many encoded items) if non-empty, terminated by a
/// zero-count block either way. A single block is valid per the Avro
/// spec, not a simplification of it.
pub fn encode_string_array(items: &[String], out: &mut Vec<u8>) {
    if !items.is_empty() {
        encode_long(items.len() as i64, out);
        for item in items {
            encode_string(item, out);
        }
    }
    encode_long(0, out);
}

/// Decodes an Avro `array<string>` starting at `*pos`, advancing it
/// past the bytes consumed. Also accepts the spec's
/// negative-count-with-byte-size block variant for robustness, even
/// though [`encode_string_array`] never emits one itself.
pub fn decode_string_array(bytes: &[u8], pos: &mut usize) -> Result<Vec<String>, AvroDecodeError> {
    let mut result = Vec::new();
    loop {
        let count = decode_long(bytes, pos)?;
        if count == 0 {
            break;
        }
        let item_count = if count < 0 {
            decode_long(bytes, pos)?; // block byte-size, unused by an item-by-item decode
            -count
        } else {
            count
        };
        for _ in 0..item_count {
            result.push(decode_string(bytes, pos)?);
        }
    }
    Ok(result)
}

/// Encodes an Avro `double`: 8 bytes, little-endian IEEE 754.
pub fn encode_double(value: f64, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Decodes an Avro `double` starting at `*pos`, advancing it past the
/// 8 bytes consumed.
pub fn decode_double(bytes: &[u8], pos: &mut usize) -> Result<f64, AvroDecodeError> {
    let end = *pos + 8;
    let slice = bytes
        .get(*pos..end)
        .ok_or(AvroDecodeError::UnexpectedEofInDouble)?;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(slice);
    *pos = end;
    Ok(f64::from_le_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_round_trips_negative_zero_and_positive_values() {
        for value in [i64::MIN, -1_000_000, -1, 0, 1, 1_000_000, i64::MAX] {
            let mut buf = Vec::new();
            encode_long(value, &mut buf);
            let mut pos = 0;
            assert_eq!(decode_long(&buf, &mut pos).unwrap(), value);
            assert_eq!(pos, buf.len());
        }
    }

    #[test]
    fn string_round_trips_including_multibyte_utf8() {
        for s in [
            "",
            "hello",
            "héllo wörld",
            "manpower.personnel-lifecycle.assignments",
        ] {
            let mut buf = Vec::new();
            encode_string(s, &mut buf);
            let mut pos = 0;
            assert_eq!(decode_string(&buf, &mut pos).unwrap(), s);
            assert_eq!(pos, buf.len());
        }
    }

    #[test]
    fn string_array_round_trips_empty_and_populated() {
        for items in [
            vec![],
            vec!["a".to_string()],
            vec!["a".to_string(), "b".to_string()],
        ] {
            let mut buf = Vec::new();
            encode_string_array(&items, &mut buf);
            let mut pos = 0;
            assert_eq!(decode_string_array(&buf, &mut pos).unwrap(), items);
            assert_eq!(pos, buf.len());
        }
    }

    #[test]
    fn double_round_trips_including_negative_and_fractional_values() {
        for value in [0.0, -1.5, 100.0, 0.75, f64::MAX, f64::MIN] {
            let mut buf = Vec::new();
            encode_double(value, &mut buf);
            let mut pos = 0;
            assert_eq!(decode_double(&buf, &mut pos).unwrap(), value);
            assert_eq!(pos, buf.len());
        }
    }

    #[test]
    fn decode_string_rejects_a_negative_length() {
        let mut buf = Vec::new();
        encode_long(-1, &mut buf);
        let mut pos = 0;
        assert_eq!(
            decode_string(&buf, &mut pos),
            Err(AvroDecodeError::NegativeLength(-1))
        );
    }

    #[test]
    fn decode_double_rejects_truncated_input() {
        let buf = [0u8; 4];
        let mut pos = 0;
        assert_eq!(
            decode_double(&buf, &mut pos),
            Err(AvroDecodeError::UnexpectedEofInDouble)
        );
    }
}
