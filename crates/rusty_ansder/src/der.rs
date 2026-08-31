//! Distinguished/Basic Encoding Rules (ITU-T X.690) — ASN.1 BER/DER TLV encoder and decoder.
//!
//! Built on top of [`rusty_wire::Reader`] and [`rusty_wire::Writer`]. Performs
//! safe, bounds-checked ASN.1 TLV parsing, integer decoding, OCTET STRING handling,
//! BOOLEAN encoding, and constructed SEQUENCE iteration.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{format, string::String, string::ToString, vec::Vec};

use core::fmt;
use rusty_wire::{Reader, Writer};

/// Result type for ASN.1 DER operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors that can occur during ASN.1 BER/DER decoding or encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Wire error from underlying cursor.
    Wire(rusty_wire::Error),
    /// Invalid value or mismatched tag.
    InvalidValue {
        /// Name of the field or tag.
        field: &'static str,
        /// Detail string.
        value: String,
    },
    /// Invalid length encoding.
    InvalidLength {
        /// Field name.
        field: &'static str,
        /// Length value.
        length: usize,
    },
}

impl From<rusty_wire::Error> for Error {
    fn from(e: rusty_wire::Error) -> Self {
        Error::Wire(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Wire(e) => write!(f, "wire error: {e}"),
            Error::InvalidValue { field, value } => write!(f, "invalid ASN.1 value for {field}: {value}"),
            Error::InvalidLength { field, length } => write!(f, "invalid ASN.1 length for {field}: {length}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

/// Universal tag: BOOLEAN (0x01).
pub const TAG_BOOLEAN: &[u8] = &[0x01];
/// Universal tag: INTEGER (0x02).
pub const TAG_INTEGER: &[u8] = &[0x02];
/// Universal tag: OCTET STRING (0x04).
pub const TAG_OCTET_STRING: &[u8] = &[0x04];
/// Universal tag: NULL (0x05).
pub const TAG_NULL: &[u8] = &[0x05];
/// Universal tag: OBJECT IDENTIFIER (0x06).
pub const TAG_OID: &[u8] = &[0x06];
/// Universal tag: ENUMERATED (0x0A).
pub const TAG_ENUMERATED: &[u8] = &[0x0A];
/// Universal tag: SEQUENCE (constructed, 0x30).
pub const TAG_SEQUENCE: &[u8] = &[0x30];
/// Universal tag: SET (constructed, 0x31).
pub const TAG_SET: &[u8] = &[0x31];

/// Write a definite-form length.
pub fn write_length(w: &mut Writer, length: usize) {
    if length < 0x80 {
        w.write_u8(length as u8);
        return;
    }
    let bytes = length.to_be_bytes();
    let start = bytes
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(bytes.len() - 1);
    let significant = &bytes[start..];
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
            field: "DER length",
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
pub fn expect_tag(r: &mut Reader<'_>, tag: &[u8]) -> Result<usize> {
    let actual = r.read_bytes(tag.len())?;
    if actual != tag {
        return Err(Error::InvalidValue {
            field: "DER tag",
            value: format!("{actual:02X?} (expected {tag:02X?})"),
        });
    }
    read_length(r)
}

/// Write an INTEGER holding an unsigned 32-bit value, minimally encoded.
pub fn write_integer_u32(w: &mut Writer, value: u32) {
    let bytes = value.to_be_bytes();
    let start = bytes
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(bytes.len() - 1);
    let significant = &bytes[start..];
    w.write_bytes(TAG_INTEGER);
    if significant[0] & 0x80 != 0 {
        write_length(w, significant.len() + 1);
        w.write_u8(0x00);
    } else {
        write_length(w, significant.len());
    }
    w.write_bytes(significant);
}

/// Read an INTEGER as an unsigned 32-bit value.
pub fn read_integer_u32(r: &mut Reader<'_>) -> Result<u32> {
    let len = expect_tag(r, TAG_INTEGER)?;
    if len == 0 || len > 5 {
        return Err(Error::InvalidLength {
            field: "DER integer",
            length: len,
        });
    }
    let bytes = r.read_bytes(len)?;
    let mut val = 0u32;
    for &b in bytes {
        val = (val << 8) | (b as u32);
    }
    Ok(val)
}

/// Write a BOOLEAN (`0xFF` for true, `0x00` for false).
pub fn write_boolean(w: &mut Writer, value: bool) {
    write_tlv(w, TAG_BOOLEAN, &[if value { 0xFF } else { 0x00 }]);
}

/// Read a BOOLEAN (any non-zero content byte is true).
pub fn read_boolean(r: &mut Reader<'_>) -> Result<bool> {
    let len = expect_tag(r, TAG_BOOLEAN)?;
    if len != 1 {
        return Err(Error::InvalidLength {
            field: "DER boolean",
            length: len,
        });
    }
    Ok(r.read_u8()? != 0)
}

/// Write an OCTET STRING.
pub fn write_octet_string(w: &mut Writer, bytes: &[u8]) {
    write_tlv(w, TAG_OCTET_STRING, bytes);
}

/// Read an OCTET STRING.
pub fn read_octet_string<'a>(r: &mut Reader<'a>) -> Result<&'a [u8]> {
    let len = expect_tag(r, TAG_OCTET_STRING)?;
    Ok(r.read_bytes(len)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_roundtrip() {
        let mut w = Writer::new();
        write_boolean(&mut w, true);
        write_boolean(&mut w, false);
        let bytes = w.into_vec();

        let mut r = Reader::new(&bytes);
        assert_eq!(read_boolean(&mut r).unwrap(), true);
        assert_eq!(read_boolean(&mut r).unwrap(), false);
        assert!(r.is_empty());
    }

    #[test]
    fn integer_u32_roundtrip() {
        let mut w = Writer::new();
        write_integer_u32(&mut w, 42);
        write_integer_u32(&mut w, 255);
        write_integer_u32(&mut w, 65535);
        let bytes = w.into_vec();

        let mut r = Reader::new(&bytes);
        assert_eq!(read_integer_u32(&mut r).unwrap(), 42);
        assert_eq!(read_integer_u32(&mut r).unwrap(), 255);
        assert_eq!(read_integer_u32(&mut r).unwrap(), 65535);
        assert!(r.is_empty());
    }

    #[test]
    fn octet_string_roundtrip() {
        let mut w = Writer::new();
        write_octet_string(&mut w, b"hello world");
        let bytes = w.into_vec();

        let mut r = Reader::new(&bytes);
        assert_eq!(read_octet_string(&mut r).unwrap(), b"hello world");
        assert!(r.is_empty());
    }
}
