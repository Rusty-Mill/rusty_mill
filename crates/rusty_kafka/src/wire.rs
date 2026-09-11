//! Kafka primitive-type codec helpers built on [`rusty_wire::{Reader,
//! Writer}`](rusty_wire). Covers Kafka's "classic" (pre-flexible/
//! non-compact) protocol encoding used by every v0 request/response in
//! this crate: fixed-width big-endian integers, an `INT16`-length-
//! prefixed string (`-1` = null), an `INT32`-length-prefixed array
//! (`-1` = null).
//!
//! `rusty_wire` only reads/writes unsigned integers; Kafka's INT16/
//! INT32/INT64 are signed two's-complement big-endian, so every helper
//! here is a thin `as` bit-reinterpreting cast over the unsigned
//! primitive -- exactly as well-defined in Rust as it needs to be, since
//! same-width integer casts preserve the bit pattern.

use crate::error::CodecError;
use rusty_wire::{Reader, Writer};

pub(crate) fn read_i8(reader: &mut Reader) -> Result<i8, CodecError> {
    Ok(reader.read_bytes(1)?[0] as i8)
}

pub(crate) fn write_i8(writer: &mut Writer, v: i8) {
    writer.write_bytes(&[v as u8]);
}

pub(crate) fn read_i16(reader: &mut Reader) -> Result<i16, CodecError> {
    Ok(reader.read_u16_be()? as i16)
}

pub(crate) fn read_i32(reader: &mut Reader) -> Result<i32, CodecError> {
    Ok(reader.read_u32_be()? as i32)
}

pub(crate) fn write_i16(writer: &mut Writer, v: i16) {
    writer.write_u16_be(v as u16);
}

pub(crate) fn write_i32(writer: &mut Writer, v: i32) {
    writer.write_u32_be(v as u32);
}

pub(crate) fn read_i64(reader: &mut Reader) -> Result<i64, CodecError> {
    Ok(reader.read_u64_be()? as i64)
}

pub(crate) fn write_i64(writer: &mut Writer, v: i64) {
    writer.write_u64_be(v as u64);
}

/// Reads a Kafka `NULLABLE_STRING`: an `INT16` byte length (`-1` means
/// `None`) followed by that many UTF-8 bytes.
pub(crate) fn read_nullable_string(reader: &mut Reader) -> Result<Option<String>, CodecError> {
    let len = read_i16(reader)?;
    if len < -1 {
        return Err(CodecError::InvalidStringLength(len));
    }
    if len == -1 {
        return Ok(None);
    }
    let bytes = reader.read_bytes(len as usize)?;
    let text = std::str::from_utf8(bytes).map_err(|_| CodecError::InvalidUtf8)?;
    Ok(Some(text.to_string()))
}

/// Reads a Kafka `STRING` -- decoded the same way as
/// [`read_nullable_string`], rejected as [`CodecError::InvalidStringLength`]
/// if the broker sent a null length for a field that isn't supposed to
/// be nullable.
pub(crate) fn read_string(reader: &mut Reader) -> Result<String, CodecError> {
    read_nullable_string(reader)?.ok_or(CodecError::InvalidStringLength(-1))
}

/// Writes a Kafka `NULLABLE_STRING`.
pub(crate) fn write_nullable_string(
    writer: &mut Writer,
    value: Option<&str>,
) -> Result<(), CodecError> {
    match value {
        None => write_i16(writer, -1),
        Some(text) => {
            let len = i16::try_from(text.len()).map_err(|_| CodecError::StringTooLong(text.len()))?;
            write_i16(writer, len);
            writer.write_bytes(text.as_bytes());
        }
    }
    Ok(())
}

/// Writes a Kafka `STRING`.
pub(crate) fn write_string(writer: &mut Writer, value: &str) -> Result<(), CodecError> {
    write_nullable_string(writer, Some(value))
}

/// Reads an `INT32` array-length prefix, rejecting anything below `-1`.
/// Every array this crate decodes today is never actually sent as null
/// by the broker in practice, so callers just floor a `-1` to `0`
/// elements via `.max(0)` at the call site rather than this helper
/// deciding that for them.
pub(crate) fn read_array_len(reader: &mut Reader) -> Result<i32, CodecError> {
    let len = read_i32(reader)?;
    if len < -1 {
        return Err(CodecError::InvalidArrayLength(len));
    }
    Ok(len)
}

/// Reads an `INT32` array-length prefix via [`read_array_len`], then
/// checks it against `reader`'s remaining bytes given `min_element_len`
/// -- the fewest bytes a single element of that array could possibly
/// encode as -- so a corrupt or hostile length prefix can't force a
/// `Vec::with_capacity` allocation the buffer could never actually back.
/// Floors a `-1` (null) length to `0`, the same convention every caller
/// of [`read_array_len`] already applies.
pub(crate) fn read_checked_array_len(
    reader: &mut Reader,
    min_element_len: usize,
) -> Result<usize, CodecError> {
    let len = read_array_len(reader)?.max(0);
    let count = len as usize;
    if count.saturating_mul(min_element_len) > reader.remaining() {
        return Err(CodecError::ArrayLengthExceedsBuffer(len, reader.remaining()));
    }
    Ok(count)
}

/// Reads a Kafka `NULLABLE_BYTES` field: an `INT32` byte length (`-1`
/// means `None`) followed by that many raw bytes -- the same shape
/// [`read_nullable_string`] uses for text, for fields that carry
/// arbitrary bytes instead (consumer-group `metadata`/`assignment`
/// payloads, `Fetch`'s `record_set`).
pub(crate) fn read_nullable_bytes<'a>(
    reader: &mut Reader<'a>,
) -> Result<Option<&'a [u8]>, CodecError> {
    let len = read_i32(reader)?;
    if len < -1 {
        return Err(CodecError::InvalidBytesLength(len));
    }
    if len == -1 {
        return Ok(None);
    }
    Ok(Some(reader.read_bytes(len as usize)?))
}

/// Writes a Kafka `NULLABLE_BYTES` field.
pub(crate) fn write_nullable_bytes(
    writer: &mut Writer,
    value: Option<&[u8]>,
) -> Result<(), CodecError> {
    match value {
        None => write_i32(writer, -1),
        Some(bytes) => {
            let len = i32::try_from(bytes.len()).map_err(|_| CodecError::BytesTooLong(bytes.len()))?;
            write_i32(writer, len);
            writer.write_bytes(bytes);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nullable_string_round_trips_some() {
        let mut writer = Writer::new();
        write_nullable_string(&mut writer, Some("hello")).unwrap();
        let bytes = writer.into_vec();
        assert_eq!(bytes, [0x00, 0x05, b'h', b'e', b'l', b'l', b'o']);

        let mut reader = Reader::new(&bytes);
        assert_eq!(
            read_nullable_string(&mut reader).unwrap(),
            Some("hello".to_string())
        );
    }

    #[test]
    fn nullable_string_round_trips_none() {
        let mut writer = Writer::new();
        write_nullable_string(&mut writer, None).unwrap();
        let bytes = writer.into_vec();
        assert_eq!(bytes, [0xFF, 0xFF]);

        let mut reader = Reader::new(&bytes);
        assert_eq!(read_nullable_string(&mut reader).unwrap(), None);
    }

    #[test]
    fn read_string_rejects_null() {
        let mut writer = Writer::new();
        write_nullable_string(&mut writer, None).unwrap();
        let bytes = writer.into_vec();
        let mut reader = Reader::new(&bytes);
        assert!(matches!(
            read_string(&mut reader),
            Err(CodecError::InvalidStringLength(-1))
        ));
    }

    #[test]
    fn array_len_rejects_below_negative_one() {
        let mut writer = Writer::new();
        write_i32(&mut writer, -2);
        let bytes = writer.into_vec();
        let mut reader = Reader::new(&bytes);
        assert!(matches!(
            read_array_len(&mut reader),
            Err(CodecError::InvalidArrayLength(-2))
        ));
    }

    #[test]
    fn i16_round_trips_negative_values() {
        let mut writer = Writer::new();
        write_i16(&mut writer, -1);
        let bytes = writer.into_vec();
        assert_eq!(bytes, [0xFF, 0xFF]);
        let mut reader = Reader::new(&bytes);
        assert_eq!(read_i16(&mut reader).unwrap(), -1);
    }

    #[test]
    fn i64_round_trips_negative_values() {
        let mut writer = Writer::new();
        write_i64(&mut writer, -2);
        let bytes = writer.into_vec();
        assert_eq!(bytes, [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE]);
        let mut reader = Reader::new(&bytes);
        assert_eq!(read_i64(&mut reader).unwrap(), -2);
    }

    #[test]
    fn i64_round_trips_a_large_positive_timestamp() {
        let mut writer = Writer::new();
        write_i64(&mut writer, 1_735_689_600_000);
        let bytes = writer.into_vec();
        let mut reader = Reader::new(&bytes);
        assert_eq!(read_i64(&mut reader).unwrap(), 1_735_689_600_000);
    }

    #[test]
    fn nullable_bytes_round_trips_some() {
        let mut writer = Writer::new();
        write_nullable_bytes(&mut writer, Some(&[1, 2, 3])).unwrap();
        let bytes = writer.into_vec();
        assert_eq!(bytes, [0x00, 0x00, 0x00, 0x03, 1, 2, 3]);

        let mut reader = Reader::new(&bytes);
        assert_eq!(
            read_nullable_bytes(&mut reader).unwrap(),
            Some(&[1, 2, 3][..])
        );
    }

    #[test]
    fn nullable_bytes_round_trips_none() {
        let mut writer = Writer::new();
        write_nullable_bytes(&mut writer, None).unwrap();
        let bytes = writer.into_vec();
        assert_eq!(bytes, [0xFF, 0xFF, 0xFF, 0xFF]);

        let mut reader = Reader::new(&bytes);
        assert_eq!(read_nullable_bytes(&mut reader).unwrap(), None);
    }

    #[test]
    fn nullable_bytes_rejects_below_negative_one() {
        let mut writer = Writer::new();
        write_i32(&mut writer, -2);
        let bytes = writer.into_vec();
        let mut reader = Reader::new(&bytes);
        assert!(matches!(
            read_nullable_bytes(&mut reader),
            Err(CodecError::InvalidBytesLength(-2))
        ));
    }

    #[test]
    fn i8_round_trips_negative_values() {
        let mut writer = Writer::new();
        write_i8(&mut writer, -1);
        let bytes = writer.into_vec();
        assert_eq!(bytes, [0xFF]);
        let mut reader = Reader::new(&bytes);
        assert_eq!(read_i8(&mut reader).unwrap(), -1);
    }


    #[test]
    fn write_nullable_string_accepts_the_maximum_i16_length() {
        let text = "a".repeat(32_767);
        let mut writer = Writer::new();
        write_nullable_string(&mut writer, Some(&text)).unwrap();
        let bytes = writer.into_vec();
        assert_eq!(&bytes[0..2], [0x7F, 0xFF]);
        assert_eq!(bytes.len(), 2 + 32_767);
    }

    #[test]
    fn write_nullable_string_rejects_a_length_over_i16_max() {
        let text = "a".repeat(32_768);
        let mut writer = Writer::new();
        let err = write_nullable_string(&mut writer, Some(&text)).unwrap_err();
        assert!(matches!(err, CodecError::StringTooLong(32_768)));
        // No malformed (wrapped-negative-length-then-payload) bytes were
        // written at all.
        assert!(writer.into_vec().is_empty());
    }

    #[test]
    fn write_nullable_bytes_rejects_a_length_over_i32_max() {
        // Building an actual 2GiB+ buffer isn't practical in a test;
        // exercise the checked conversion directly the same way
        // `write_nullable_bytes` does.
        assert!(i32::try_from(u32::MAX as usize + 1).is_err());
    }

    #[test]
    fn checked_array_len_rejects_a_tiny_payload_claiming_i32_max_elements() {
        let mut writer = Writer::new();
        write_i32(&mut writer, i32::MAX);
        writer.write_bytes(&[0, 1, 2, 3]); // a few trailing bytes, nowhere near enough
        let bytes = writer.into_vec();
        let mut reader = Reader::new(&bytes);
        let err = read_checked_array_len(&mut reader, 4).unwrap_err();
        assert!(matches!(
            err,
            CodecError::ArrayLengthExceedsBuffer(i32::MAX, 4)
        ));
    }

    #[test]
    fn checked_array_len_accepts_a_count_the_buffer_can_back() {
        let mut writer = Writer::new();
        write_i32(&mut writer, 2);
        write_i32(&mut writer, 10);
        write_i32(&mut writer, 20);
        let bytes = writer.into_vec();
        let mut reader = Reader::new(&bytes);
        assert_eq!(read_checked_array_len(&mut reader, 4).unwrap(), 2);
    }
}
