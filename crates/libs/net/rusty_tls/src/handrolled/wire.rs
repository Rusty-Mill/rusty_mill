//! TLS wire primitives — stage 3b.
//!
//! TLS's presentation language (RFC 8446 §3) is a handful of fixed-width
//! integers and length-prefixed vectors. This module is those, read and
//! written strictly.
//!
//! # Why it is separate from [`super::der`]
//!
//! They solve the same shape of problem — reading attacker-supplied,
//! length-prefixed structures — and they are deliberately not shared, because
//! the encodings have nothing in common beyond that shape. DER is
//! tag-length-value with canonicality rules; TLS is bare length prefixes of
//! one, two, or three octets with no tags at all. A reader abstracted over
//! both would be an abstraction over a coincidence.
//!
//! What *is* shared is the discipline: every length is checked before use,
//! nothing recurses on input structure, and a sub-reader must be finished
//! explicitly so that "I read the fields I wanted" cannot pass for "this was
//! exactly that structure".
//!
//! # Exact consumption is the point
//!
//! [`Reader::finish`] erroring on leftover bytes is what makes a
//! length-prefixed parse meaningful. Without it, a vector whose contents are
//! longer than the fields inside it parses "successfully" while the extra
//! bytes go unexamined — which is where an attacker puts things.

use core::fmt;

/// Everything the wire reader can refuse.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireError {
    /// The input ended before a value was complete.
    UnexpectedEnd {
        /// How many bytes were needed.
        needed: usize,
        /// How many were left.
        available: usize,
    },
    /// Bytes remained after the last value a structure defines.
    TrailingData {
        /// How many bytes were left over.
        remaining: usize,
    },
    /// A length prefix described more bytes than the enclosing structure has.
    ///
    /// Distinct from [`WireError::UnexpectedEnd`]: the input did not merely
    /// run out, it claimed a size its container cannot hold, which is a
    /// contradiction rather than a truncation.
    LengthOverrun {
        /// The length the prefix declared.
        declared: usize,
        /// The bytes actually available.
        available: usize,
    },
    /// A vector was empty where the grammar requires at least one element.
    EmptyVector,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd { needed, available } => {
                write!(f, "needed {needed} bytes, {available} available")
            }
            Self::TrailingData { remaining } => {
                write!(f, "{remaining} trailing bytes after the final field")
            }
            Self::LengthOverrun {
                declared,
                available,
            } => write!(
                f,
                "a length prefix declares {declared} bytes, {available} available"
            ),
            Self::EmptyVector => f.write_str("an empty vector where one element is required"),
        }
    }
}

impl std::error::Error for WireError {}

type Result<T> = core::result::Result<T, WireError>;

/// A cursor over a TLS-encoded byte slice.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    /// Start reading at the beginning of `input`.
    pub const fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    /// True when every byte has been consumed.
    pub const fn is_empty(&self) -> bool {
        self.position >= self.input.len()
    }

    /// How many bytes remain.
    pub const fn remaining(&self) -> usize {
        self.input.len() - self.position
    }

    /// Consume the reader, erroring if anything is left.
    ///
    /// Call this at the end of every structure — see the module docs on why
    /// it is what makes a length-prefixed parse mean anything.
    pub fn finish(self) -> Result<()> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(WireError::TrailingData {
                remaining: self.remaining(),
            })
        }
    }

    /// Read `count` bytes.
    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(WireError::UnexpectedEnd {
                needed: count,
                available: self.remaining(),
            })?;
        let slice = self
            .input
            .get(self.position..end)
            .ok_or(WireError::UnexpectedEnd {
                needed: count,
                available: self.remaining(),
            })?;
        self.position = end;
        Ok(slice)
    }

    /// Read a `uint8`.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// Read a `uint16`, big-endian as everything in TLS is.
    pub fn u16(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// Read a `uint24` — the length prefix on a handshake message.
    pub fn u24(&mut self) -> Result<u32> {
        let bytes = self.take(3)?;
        Ok(u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]))
    }

    /// Read a `uint32`.
    pub fn u32(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a vector with a one-octet length prefix, returning its contents.
    pub fn vector_u8(&mut self) -> Result<&'a [u8]> {
        let length = usize::from(self.u8()?);
        self.take_checked(length)
    }

    /// Read a vector with a two-octet length prefix.
    pub fn vector_u16(&mut self) -> Result<&'a [u8]> {
        let length = usize::from(self.u16()?);
        self.take_checked(length)
    }

    /// Read a vector with a three-octet length prefix.
    pub fn vector_u24(&mut self) -> Result<&'a [u8]> {
        let length = self.u24()? as usize;
        self.take_checked(length)
    }

    /// A vector's contents as their own reader, so the enclosing structure
    /// cannot accidentally read past the vector's end.
    ///
    /// The returned reader is independent: nesting in the input becomes
    /// another loop, never another stack frame.
    pub fn sub_u16(&mut self) -> Result<Reader<'a>> {
        Ok(Reader::new(self.vector_u16()?))
    }

    /// As [`Reader::sub_u16`], for a one-octet prefix.
    pub fn sub_u8(&mut self) -> Result<Reader<'a>> {
        Ok(Reader::new(self.vector_u8()?))
    }

    /// As [`Reader::sub_u16`], for a three-octet prefix.
    pub fn sub_u24(&mut self) -> Result<Reader<'a>> {
        Ok(Reader::new(self.vector_u24()?))
    }

    /// The distinction between "ran out" and "claimed more than the container
    /// holds".
    ///
    /// This is not what keeps a read inside its buffer — [`Reader::take`] is
    /// bounds-checked on its own, and deleting this check cannot let anything
    /// escape. It exists only to name the failure, and that is worth a branch:
    /// a truncated stream is a network event, while a length prefix claiming
    /// more than its container holds is a peer sending a contradiction. A
    /// reader that reports the second as the first reports a hostile peer as
    /// a flaky link.
    ///
    /// Because it changes no accept/reject decision, message-level tests
    /// cannot pin it — `tests/handrolled_wire.rs` does, and says why.
    fn take_checked(&mut self, length: usize) -> Result<&'a [u8]> {
        if length > self.remaining() {
            return Err(WireError::LengthOverrun {
                declared: length,
                available: self.remaining(),
            });
        }
        self.take(length)
    }
}

/// A buffer that encodes TLS's presentation language.
///
/// Length prefixes are written by [`Writer::vector_u8`] and friends, which
/// take a closure and backfill the length once its contents are known —
/// so a prefix can never disagree with what follows it, because nothing
/// writes one by hand.
#[derive(Clone, Debug, Default)]
pub struct Writer {
    buffer: Vec<u8>,
}

impl Writer {
    /// A new, empty writer.
    pub const fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// The bytes written so far.
    pub fn as_slice(&self) -> &[u8] {
        &self.buffer
    }

    /// Consume the writer for its bytes.
    pub fn into_vec(self) -> Vec<u8> {
        self.buffer
    }

    /// How many bytes have been written.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// True when nothing has been written.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Append a `uint8`.
    pub fn u8(&mut self, value: u8) {
        self.buffer.push(value);
    }

    /// Append a `uint16`.
    pub fn u16(&mut self, value: u16) {
        self.buffer.extend_from_slice(&value.to_be_bytes());
    }

    /// Append a `uint24`.
    pub fn u24(&mut self, value: u32) {
        debug_assert!(value < 0x0100_0000, "a uint24 holds 24 bits");
        self.buffer.extend_from_slice(&value.to_be_bytes()[1..]);
    }

    /// Append a `uint32`.
    pub fn u32(&mut self, value: u32) {
        self.buffer.extend_from_slice(&value.to_be_bytes());
    }

    /// Append raw bytes.
    pub fn bytes(&mut self, value: &[u8]) {
        self.buffer.extend_from_slice(value);
    }

    /// Write a one-octet-prefixed vector whose contents `body` produces.
    pub fn vector_u8(&mut self, body: impl FnOnce(&mut Self)) {
        let start = self.buffer.len();
        self.buffer.push(0);
        body(self);
        let length = self.buffer.len() - start - 1;
        debug_assert!(length <= u8::MAX as usize, "vector fits a one-octet prefix");
        self.buffer[start] = length as u8;
    }

    /// Write a two-octet-prefixed vector.
    pub fn vector_u16(&mut self, body: impl FnOnce(&mut Self)) {
        let start = self.buffer.len();
        self.buffer.extend_from_slice(&[0, 0]);
        body(self);
        let length = self.buffer.len() - start - 2;
        debug_assert!(
            length <= u16::MAX as usize,
            "vector fits a two-octet prefix"
        );
        self.buffer[start..start + 2].copy_from_slice(&(length as u16).to_be_bytes());
    }

    /// Write a three-octet-prefixed vector.
    pub fn vector_u24(&mut self, body: impl FnOnce(&mut Self)) {
        let start = self.buffer.len();
        self.buffer.extend_from_slice(&[0, 0, 0]);
        body(self);
        let length = self.buffer.len() - start - 3;
        debug_assert!(length < 0x0100_0000, "vector fits a three-octet prefix");
        self.buffer[start..start + 3].copy_from_slice(&(length as u32).to_be_bytes()[1..]);
    }
}
