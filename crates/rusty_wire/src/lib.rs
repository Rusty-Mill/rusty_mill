//! Bounds-checked reading and writing of primitive integers and byte slices.
//!
//! Protocols frequently mix endianness: transport layers (e.g. TPKT, X.224, SSH) are
//! big-endian while inner payload structures may be little-endian (e.g. RDP).
//! Rather than sprinkle `from_be_bytes` / `from_le_bytes` throughout a codec,
//! all primitive access goes through [`Reader`] and [`Writer`], which keep endianness
//! explicit at every call site and guarantee safe, bounds-checked operations.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{string::String, string::ToString, vec::Vec};

use core::fmt;

/// Result type for cursor operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors that can occur during byte cursor reading or writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Attempted to read more bytes than available.
    UnexpectedEof {
        /// Number of bytes requested.
        needed: usize,
        /// Number of bytes remaining.
        available: usize,
    },
    /// An offset or value was invalid.
    InvalidValue {
        /// Name of the field.
        field: &'static str,
        /// Value representation.
        value: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnexpectedEof { needed, available } => {
                write!(
                    f,
                    "unexpected EOF: needed {needed} bytes, but only {available} available"
                )
            }
            Error::InvalidValue { field, value } => {
                write!(f, "invalid value for {field}: {value}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

/// A forward-only cursor over a byte slice.
///
/// Every read advances the internal position and is checked against the
/// remaining length, returning [`Error::UnexpectedEof`] rather than panicking.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Wrap a byte slice for reading.
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    /// Number of bytes not yet consumed.
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Current offset from the start of the underlying slice.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Returns `true` when every byte has been consumed.
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn ensure(&self, needed: usize) -> Result<()> {
        if self.remaining() < needed {
            return Err(Error::UnexpectedEof {
                needed,
                available: self.remaining(),
            });
        }
        Ok(())
    }

    /// Read `len` bytes without copying, advancing the cursor.
    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        self.ensure(len)?;
        let out = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(out)
    }

    /// Read exactly `N` bytes into a fixed-size array, advancing the cursor.
    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let b = self.read_bytes(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(b);
        Ok(out)
    }

    /// Borrow the remaining bytes without advancing the cursor.
    pub fn peek_remaining(&self) -> &'a [u8] {
        &self.buf[self.pos..]
    }

    /// Read a single unsigned byte.
    pub fn read_u8(&mut self) -> Result<u8> {
        self.ensure(1)?;
        let b = self.buf[self.pos];
        self.pos += 1;
        Ok(b)
    }

    /// Read a big-endian `u16`.
    pub fn read_u16_be(&mut self) -> Result<u16> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    /// Read a little-endian `u16`.
    pub fn read_u16_le(&mut self) -> Result<u16> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    /// Read a big-endian `u32`.
    pub fn read_u32_be(&mut self) -> Result<u32> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Read a little-endian `u32`.
    pub fn read_u32_le(&mut self) -> Result<u32> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Read a big-endian `u64`.
    pub fn read_u64_be(&mut self) -> Result<u64> {
        let b = self.read_bytes(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// Read a little-endian `u64`.
    pub fn read_u64_le(&mut self) -> Result<u64> {
        let b = self.read_bytes(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// Skip `len` bytes, advancing the cursor.
    pub fn skip(&mut self, len: usize) -> Result<()> {
        self.ensure(len)?;
        self.pos += len;
        Ok(())
    }
}

/// A growable byte buffer with explicit-endianness integer writes.
#[derive(Debug, Default, Clone)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// Create an empty writer.
    pub fn new() -> Self {
        Writer { buf: Vec::new() }
    }

    /// Create an empty writer with room for `cap` bytes.
    pub fn with_capacity(cap: usize) -> Self {
        Writer {
            buf: Vec::with_capacity(cap),
        }
    }

    /// Number of bytes written so far.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns `true` when nothing has been written.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Append raw bytes.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Append a single byte.
    pub fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    /// Append a big-endian `u16`.
    pub fn write_u16_be(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    /// Append a little-endian `u16`.
    pub fn write_u16_le(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Append a big-endian `u32`.
    pub fn write_u32_be(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    /// Append a little-endian `u32`.
    pub fn write_u32_le(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Append a big-endian `u64`.
    pub fn write_u64_be(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    /// Append a little-endian `u64`.
    pub fn write_u64_le(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Overwrite two bytes at `offset` with a big-endian `u16`.
    pub fn patch_u16_be(&mut self, offset: usize, v: u16) -> Result<()> {
        if offset + 2 > self.buf.len() {
            return Err(Error::InvalidValue {
                field: "patch offset",
                value: offset.to_string(),
            });
        }
        self.buf[offset..offset + 2].copy_from_slice(&v.to_be_bytes());
        Ok(())
    }

    /// Overwrite four bytes at `offset` with a big-endian `u32`.
    pub fn patch_u32_be(&mut self, offset: usize, v: u32) -> Result<()> {
        if offset + 4 > self.buf.len() {
            return Err(Error::InvalidValue {
                field: "patch offset",
                value: offset.to_string(),
            });
        }
        self.buf[offset..offset + 4].copy_from_slice(&v.to_be_bytes());
        Ok(())
    }

    /// Consume the writer and return the accumulated bytes.
    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    /// Borrow the accumulated bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_mixed_endianness() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let mut r = Reader::new(&data);
        assert_eq!(r.read_u16_be().unwrap(), 0x0102);
        assert_eq!(r.read_u16_le().unwrap(), 0x0403);
        assert!(r.is_empty());
    }

    #[test]
    fn read_past_end_errors() {
        let data = [0x01];
        let mut r = Reader::new(&data);
        assert_eq!(
            r.read_u16_be().unwrap_err(),
            Error::UnexpectedEof {
                needed: 2,
                available: 1
            }
        );
    }

    #[test]
    fn writer_roundtrips() {
        let mut w = Writer::new();
        w.write_u8(0x03);
        w.write_u16_be(0x1234);
        w.write_u32_le(0xDEAD_BEEF);
        let bytes = w.into_vec();
        assert_eq!(bytes, [0x03, 0x12, 0x34, 0xEF, 0xBE, 0xAD, 0xDE]);
    }

    #[test]
    fn u64_roundtrips_both_endian() {
        let mut w = Writer::new();
        w.write_u64_be(0x0102_0304_0506_0708);
        w.write_u64_le(0x0102_0304_0506_0708);
        let bytes = w.into_vec();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.read_u64_be().unwrap(), 0x0102_0304_0506_0708);
        assert_eq!(r.read_u64_le().unwrap(), 0x0102_0304_0506_0708);
        assert!(r.is_empty());
    }

    #[test]
    fn patch_backfills_length() {
        let mut w = Writer::new();
        w.write_u16_be(0); // placeholder
        w.write_bytes(&[0xAA, 0xBB]);
        w.patch_u16_be(0, w.len() as u16).unwrap();
        assert_eq!(w.into_vec(), [0x00, 0x04, 0xAA, 0xBB]);
    }
}
