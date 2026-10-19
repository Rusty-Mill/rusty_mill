use crate::varint::{self, zigzag32, zigzag64};
use crate::{Error, WireType, MAX_FIELD_NUMBER};

/// The format cannot describe a message of 2 GiB or more.
const MAX_MESSAGE_LEN: usize = i32::MAX as usize;

/// Builds a message front to back. Writing never fails; [`Writer::finish`] reports an invalid
/// field number or a message past the format's 2 GiB limit.
///
/// Fields are written in call order. A proto3 default (zero, empty) is written if you ask for
/// it: skipping defaults is the schema's decision.
#[derive(Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
    err: Option<Error>,
}

impl Writer {
    /// An empty writer.
    pub fn new() -> Writer {
        Writer::default()
    }

    /// The finished message, or the first error recorded.
    pub fn finish(self) -> Result<Vec<u8>, Error> {
        self.finish_within(MAX_MESSAGE_LEN)
    }

    fn finish_within(self, max: usize) -> Result<Vec<u8>, Error> {
        match self.err {
            Some(e) => Err(e),
            None if self.buf.len() > max => Err(Error::TooLarge),
            None => Ok(self.buf),
        }
    }

    /// Writes a tag, or records [`Error::TooLarge`] and returns false for an invalid number.
    fn tag(&mut self, number: u32, wire: WireType) -> bool {
        if number == 0 || number > MAX_FIELD_NUMBER {
            self.err.get_or_insert(Error::TooLarge);
            return false;
        }
        varint::encode(u64::from(number) << 3 | wire as u64, &mut self.buf);
        true
    }

    fn fixed<const N: usize>(&mut self, number: u32, wire: WireType, bytes: [u8; N]) {
        if self.tag(number, wire) {
            self.buf.extend_from_slice(&bytes);
        }
    }

    /// A `uint64` (also the raw form of every other varint type).
    pub fn uint64(&mut self, number: u32, v: u64) {
        if self.tag(number, WireType::Varint) {
            varint::encode(v, &mut self.buf);
        }
    }

    /// A `uint32`.
    pub fn uint32(&mut self, number: u32, v: u32) {
        self.uint64(number, u64::from(v));
    }

    /// An `int32` (negative values take 10 bytes, as the format requires).
    pub fn int32(&mut self, number: u32, v: i32) {
        self.uint64(number, i64::from(v) as u64);
    }

    /// An `int64`.
    pub fn int64(&mut self, number: u32, v: i64) {
        self.uint64(number, v as u64);
    }

    /// A zigzag `sint32`.
    pub fn sint32(&mut self, number: u32, v: i32) {
        self.uint64(number, u64::from(zigzag32(v)));
    }

    /// A zigzag `sint64`.
    pub fn sint64(&mut self, number: u32, v: i64) {
        self.uint64(number, zigzag64(v));
    }

    /// A `bool`.
    pub fn bool(&mut self, number: u32, v: bool) {
        self.uint64(number, u64::from(v));
    }

    /// A `fixed32`.
    pub fn fixed32(&mut self, number: u32, v: u32) {
        self.fixed(number, WireType::Fixed32, v.to_le_bytes());
    }

    /// An `sfixed32`.
    pub fn sfixed32(&mut self, number: u32, v: i32) {
        self.fixed(number, WireType::Fixed32, v.to_le_bytes());
    }

    /// A `float`.
    pub fn float(&mut self, number: u32, v: f32) {
        self.fixed(number, WireType::Fixed32, v.to_le_bytes());
    }

    /// A `fixed64`.
    pub fn fixed64(&mut self, number: u32, v: u64) {
        self.fixed(number, WireType::Fixed64, v.to_le_bytes());
    }

    /// An `sfixed64`.
    pub fn sfixed64(&mut self, number: u32, v: i64) {
        self.fixed(number, WireType::Fixed64, v.to_le_bytes());
    }

    /// A `double`.
    pub fn double(&mut self, number: u32, v: f64) {
        self.fixed(number, WireType::Fixed64, v.to_le_bytes());
    }

    /// A `bytes` field.
    pub fn bytes(&mut self, number: u32, v: &[u8]) {
        if self.tag(number, WireType::Len) {
            varint::encode(v.len() as u64, &mut self.buf);
            self.buf.extend_from_slice(v);
        }
    }

    /// A `string` field.
    pub fn string(&mut self, number: u32, v: &str) {
        self.bytes(number, v.as_bytes());
    }

    /// A sub-message: `body` writes its fields into the writer it is given, and the length
    /// prefix is fixed up afterwards.
    pub fn message(&mut self, number: u32, body: impl FnOnce(&mut Writer)) {
        if !self.tag(number, WireType::Len) {
            return;
        }
        // One byte for the length covers bodies under 128 bytes; a longer one shifts once.
        let at = self.buf.len();
        self.buf.push(0);
        body(self);
        let len = self.buf.len() - at - 1;
        if len < 0x80 {
            self.buf[at] = len as u8;
            return;
        }
        let mut prefix = Vec::with_capacity(varint::MAX_LEN);
        varint::encode(len as u64, &mut prefix);
        drop(self.buf.splice(at..=at, prefix));
    }

    /// A packed run of varints, each already in its raw `u64` form (see [`Writer::uint64`]).
    pub fn packed_varints(&mut self, number: u32, values: impl IntoIterator<Item = u64>) {
        self.message(number, |w| {
            for v in values {
                varint::encode(v, &mut w.buf);
            }
        });
    }

    /// A packed run of 4-byte values (raw bits of `fixed32`, `sfixed32` or `float`).
    pub fn packed_fixed32(&mut self, number: u32, values: impl IntoIterator<Item = u32>) {
        self.message(number, |w| {
            for v in values {
                w.buf.extend_from_slice(&v.to_le_bytes());
            }
        });
    }

    /// A packed run of 8-byte values (raw bits of `fixed64`, `sfixed64` or `double`).
    pub fn packed_fixed64(&mut self, number: u32, values: impl IntoIterator<Item = u64>) {
        self.message(number, |w| {
            for v in values {
                w.buf.extend_from_slice(&v.to_le_bytes());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_refuses_a_message_over_the_limit() {
        let mut w = Writer::new();
        w.bytes(1, &[0; 8]);
        assert_eq!(
            w.finish_within(10),
            Ok(vec![0x0a, 8, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        let mut w = Writer::new();
        w.bytes(1, &[0; 8]);
        assert_eq!(w.finish_within(9), Err(Error::TooLarge));
    }
}
