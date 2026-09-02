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
pub(crate) fn write_nullable_string(writer: &mut Writer, value: Option<&str>) {
    match value {
        None => write_i16(writer, -1),
        Some(text) => {
            write_i16(writer, text.len() as i16);
            writer.write_bytes(text.as_bytes());
        }
    }
}

/// Writes a Kafka `STRING`.
pub(crate) fn write_string(writer: &mut Writer, value: &str) {
    write_nullable_string(writer, Some(value));
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
pub(crate) fn write_nullable_bytes(writer: &mut Writer, value: Option<&[u8]>) {
    match value {
        None => write_i32(writer, -1),
        Some(bytes) => {
            write_i32(writer, bytes.len() as i32);
            writer.write_bytes(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nullable_string_round_trips_some() {
        let mut writer = Writer::new();
        write_nullable_string(&mut writer, Some("hello"));
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
        write_nullable_string(&mut writer, None);
        let bytes = writer.into_vec();
        assert_eq!(bytes, [0xFF, 0xFF]);

        let mut reader = Reader::new(&bytes);
        assert_eq!(read_nullable_string(&mut reader).unwrap(), None);
    }

    #[test]
    fn read_string_rejects_null() {
        let mut writer = Writer::new();
        write_nullable_string(&mut writer, None);
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
        write_nullable_bytes(&mut writer, Some(&[1, 2, 3]));
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
        write_nullable_bytes(&mut writer, None);
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
}
