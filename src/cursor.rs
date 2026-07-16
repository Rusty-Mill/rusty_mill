//! Bounds-checked reading and writing of primitive integers.
//!
//! RDP mixes endianness: the transport layers (TPKT, X.224) are big-endian
//! while the RDP structures layered on top are little-endian. Rather than
//! sprinkle `from_be_bytes` / `from_le_bytes` throughout the codec, all
//! primitive access goes through [`Reader`] and [`Writer`], which keep the
//! endianness explicit at every call site and never read or write out of
//! bounds.

use crate::error::{Error, Result};

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

    /// Skip `len` bytes, advancing the cursor.
    pub fn skip(&mut self, len: usize) -> Result<()> {
        self.ensure(len)?;
        self.pos += len;
        Ok(())
    }
}

/// A growable byte buffer with explicit-endianness integer writes.
///
/// `Writer` never fails on capacity (it grows a `Vec`); the fallible methods
/// exist only where a value must fit a fixed-width protocol field.
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

    /// Overwrite two bytes at `offset` with a big-endian `u16`.
    ///
    /// Used to back-patch length fields once the final size is known.
    /// Returns [`Error::InvalidValue`] if the offset is out of range.
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
    fn patch_backfills_length() {
        let mut w = Writer::new();
        w.write_u16_be(0); // placeholder
        w.write_bytes(&[0xAA, 0xBB]);
        w.patch_u16_be(0, w.len() as u16).unwrap();
        assert_eq!(w.into_vec(), [0x00, 0x04, 0xAA, 0xBB]);
    }
}
