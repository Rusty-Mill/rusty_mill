//! Distinguished/Basic Encoding Rules (ITU-T X.690) — the subset MCS needs.
//!
//! The MCS connection PDUs (`Connect-Initial` / `Connect-Response`) are
//! BER-encoded, unlike the PER-encoded domain PDUs. RDP only exercises a
//! handful of universal types plus two application-class tags, all with the
//! definite short/long length form. That subset is implemented here.
//!
//! Tags are represented as raw byte slices so the two-byte application tags
//! (`[APPLICATION 101]` = `7F 65`, `[APPLICATION 102]` = `7F 66`) are handled
//! the same way as the single-byte universal tags.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// Universal tag: BOOLEAN.
pub const TAG_BOOLEAN: &[u8] = &[0x01];
/// Universal tag: INTEGER.
pub const TAG_INTEGER: &[u8] = &[0x02];
/// Universal tag: OCTET STRING.
pub const TAG_OCTET_STRING: &[u8] = &[0x04];
/// Universal tag: ENUMERATED.
pub const TAG_ENUMERATED: &[u8] = &[0x0A];
/// Universal tag: SEQUENCE (constructed).
pub const TAG_SEQUENCE: &[u8] = &[0x30];
/// Application tag `[APPLICATION 101]` used by `Connect-Initial`.
pub const TAG_CONNECT_INITIAL: &[u8] = &[0x7F, 0x65];
/// Application tag `[APPLICATION 102]` used by `Connect-Response`.
pub const TAG_CONNECT_RESPONSE: &[u8] = &[0x7F, 0x66];

/// Write a definite-form length.
///
/// Short form for values below 128, otherwise the long form with the minimum
/// number of trailing bytes.
pub fn write_length(w: &mut Writer, length: usize) {
    if length < 0x80 {
        w.write_u8(length as u8);
        return;
    }
    // Long form: 0x80 | number-of-following-bytes, then big-endian length.
    let mut bytes = length.to_be_bytes();
    let start = bytes
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(bytes.len() - 1);
    let significant = &mut bytes[start..];
    w.write_u8(0x80 | significant.len() as u8);
    w.write_bytes(significant);
}

/// Read a definite-form length. Indefinite form is rejected.
pub fn read_length(r: &mut Reader<'_>) -> Result<usize> {
    let first = r.read_u8()?;
    if first & 0x80 == 0 {
        return Ok(first as usize);
    }
    let count = (first & 0x7F) as usize;
    if count == 0 || count > core::mem::size_of::<usize>() {
        return Err(Error::InvalidValue {
            field: "BER length",
            value: format!("0x{first:02X}"),
        });
    }
    let mut value = 0usize;
    for _ in 0..count {
        value = (value << 8) | r.read_u8()? as usize;
    }
    Ok(value)
}

/// Write `tag` followed by a definite length and `contents`.
pub fn write_tlv(w: &mut Writer, tag: &[u8], contents: &[u8]) {
    w.write_bytes(tag);
    write_length(w, contents.len());
    w.write_bytes(contents);
}

/// Expect `tag` at the cursor and return the contents length.
///
/// Advances past the tag and length, leaving the cursor at the first content
/// byte. Returns [`Error::InvalidValue`] if the tag does not match.
pub fn expect_tag(r: &mut Reader<'_>, tag: &[u8]) -> Result<usize> {
    let actual = r.read_bytes(tag.len())?;
    if actual != tag {
        return Err(Error::InvalidValue {
            field: "BER tag",
            value: format!("{actual:02X?} (expected {tag:02X?})"),
        });
    }
    read_length(r)
}

/// Write an INTEGER holding an unsigned value, minimally encoded.
///
/// A leading `0x00` is inserted when the top bit would otherwise make the
/// value look negative, as BER integers are two's-complement.
pub fn write_integer(w: &mut Writer, value: u32) {
    let mut bytes = value.to_be_bytes();
    let start = bytes
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(bytes.len() - 1);
    let significant = &mut bytes[start..];
    w.write_bytes(TAG_INTEGER);
    if significant[0] & 0x80 != 0 {
        write_length(w, significant.len() + 1);
        w.write_u8(0x00);
    } else {
        write_length(w, significant.len());
    }
    w.write_bytes(significant);
}

/// Read an INTEGER as an unsigned value (MCS integers are non-negative).
pub fn read_integer(r: &mut Reader<'_>) -> Result<u32> {
    let len = expect_tag(r, TAG_INTEGER)?;
    read_uint_contents(r, len, "BER integer")
}

/// Read an ENUMERATED as a byte.
pub fn read_enumerated(r: &mut Reader<'_>) -> Result<u8> {
    let len = expect_tag(r, TAG_ENUMERATED)?;
    if len != 1 {
        return Err(Error::InvalidLength {
            field: "BER enumerated",
            length: len,
        });
    }
    r.read_u8()
}

/// Write an ENUMERATED byte.
pub fn write_enumerated(w: &mut Writer, value: u8) {
    write_tlv(w, TAG_ENUMERATED, &[value]);
}

/// Write a BOOLEAN (`0xFF` for true, `0x00` for false, per DER).
pub fn write_boolean(w: &mut Writer, value: bool) {
    write_tlv(w, TAG_BOOLEAN, &[if value { 0xFF } else { 0x00 }]);
}

/// Read a BOOLEAN (any non-zero content byte is true).
pub fn read_boolean(r: &mut Reader<'_>) -> Result<bool> {
    let len = expect_tag(r, TAG_BOOLEAN)?;
    if len != 1 {
        return Err(Error::InvalidLength {
            field: "BER boolean",
            length: len,
        });
    }
    Ok(r.read_u8()? != 0)
}

/// Write an OCTET STRING.
pub fn write_octet_string(w: &mut Writer, bytes: &[u8]) {
    write_tlv(w, TAG_OCTET_STRING, bytes);
}

/// Read an OCTET STRING, borrowing the content bytes.
pub fn read_octet_string<'a>(r: &mut Reader<'a>) -> Result<&'a [u8]> {
    let len = expect_tag(r, TAG_OCTET_STRING)?;
    r.read_bytes(len)
}

fn read_uint_contents(r: &mut Reader<'_>, len: usize, field: &'static str) -> Result<u32> {
    if len == 0 || len > 5 {
        return Err(Error::InvalidLength { field, length: len });
    }
    let bytes = r.read_bytes(len)?;
    // A 5-byte encoding is only valid when the leading byte is a 0x00 pad.
    let significant = if bytes.len() == 5 {
        if bytes[0] != 0 {
            return Err(Error::InvalidValue {
                field,
                value: format!("{bytes:02X?} exceeds u32"),
            });
        }
        &bytes[1..]
    } else {
        bytes
    };
    let mut value = 0u32;
    for &b in significant {
        value = (value << 8) | b as u32;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_forms_roundtrip() {
        for len in [0usize, 1, 0x7F, 0x80, 0xFF, 0x0100, 0x1234, 0x0001_0000] {
            let mut w = Writer::new();
            write_length(&mut w, len);
            let bytes = w.into_vec();
            let mut r = Reader::new(&bytes);
            assert_eq!(read_length(&mut r).unwrap(), len, "len {len:#x}");
            assert!(r.is_empty());
        }
    }

    #[test]
    fn length_long_form_encoding() {
        let mut w = Writer::new();
        write_length(&mut w, 0x0123);
        assert_eq!(w.into_vec(), [0x82, 0x01, 0x23]);
    }

    #[test]
    fn integer_inserts_sign_pad() {
        // 0x80 has the high bit set, so a leading 0x00 must be added.
        let mut w = Writer::new();
        write_integer(&mut w, 0x80);
        assert_eq!(w.as_slice(), &[0x02, 0x02, 0x00, 0x80]);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(read_integer(&mut r).unwrap(), 0x80);
    }

    #[test]
    fn integer_roundtrip_various() {
        for v in [
            0u32,
            1,
            0x7F,
            0x80,
            0xFF,
            0x0100,
            0xFFFF,
            0x00FF_FFFF,
            0xFFFF_FFFF,
        ] {
            let mut w = Writer::new();
            write_integer(&mut w, v);
            let mut r = Reader::new(w.as_slice());
            assert_eq!(read_integer(&mut r).unwrap(), v, "value {v:#x}");
        }
    }

    #[test]
    fn boolean_and_octet_string() {
        let mut w = Writer::new();
        write_boolean(&mut w, true);
        write_octet_string(&mut w, &[0xDE, 0xAD]);
        let mut r = Reader::new(w.as_slice());
        assert!(read_boolean(&mut r).unwrap());
        assert_eq!(read_octet_string(&mut r).unwrap(), &[0xDE, 0xAD]);
    }

    #[test]
    fn expect_tag_mismatch_errors() {
        let mut r = Reader::new(&[0x02, 0x01, 0x00]);
        assert!(matches!(
            expect_tag(&mut r, TAG_OCTET_STRING).unwrap_err(),
            Error::InvalidValue {
                field: "BER tag",
                ..
            }
        ));
    }

    #[test]
    fn application_tag_roundtrip() {
        let mut w = Writer::new();
        write_tlv(&mut w, TAG_CONNECT_RESPONSE, &[0xAA, 0xBB]);
        let bytes = w.into_vec();
        assert_eq!(&bytes[..2], TAG_CONNECT_RESPONSE);
        let mut r = Reader::new(&bytes);
        assert_eq!(expect_tag(&mut r, TAG_CONNECT_RESPONSE).unwrap(), 2);
        assert_eq!(r.read_bytes(2).unwrap(), &[0xAA, 0xBB]);
    }
}
