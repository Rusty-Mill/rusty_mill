//! TPKT framing (RFC 1006 / ITU-T T.123).
//!
//! RDP tunnels ISO transport PDUs inside TPKT packets over TCP. A TPKT
//! header is four bytes:
//!
//! ```text
//! +--------+--------+----------------+
//! | 0x03   | 0x00   |  length (BE)   |
//! | version| reserv |    u16         |
//! +--------+--------+----------------+
//! ```
//!
//! `length` counts the whole packet including the 4-byte header, so the
//! payload length is `length - 4`. This module only handles the framing;
//! the payload is an opaque X.224 TPDU handled by [`crate::x224`].

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// TPKT version byte. RDP always uses version 3.
pub const TPKT_VERSION: u8 = 0x03;

/// Length of a TPKT header in bytes.
pub const TPKT_HEADER_LEN: usize = 4;

/// The largest total packet a 16-bit length field can describe.
pub const TPKT_MAX_LEN: usize = u16::MAX as usize;

/// A decoded TPKT packet borrowing its payload from the input buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tpkt<'a> {
    /// The X.224 TPDU carried by this packet.
    pub payload: &'a [u8],
}

impl<'a> Tpkt<'a> {
    /// Wrap a payload for encoding.
    pub fn new(payload: &'a [u8]) -> Self {
        Tpkt { payload }
    }

    /// Total encoded size (header + payload).
    pub fn encoded_len(&self) -> usize {
        TPKT_HEADER_LEN + self.payload.len()
    }

    /// Encode this packet, appending header and payload to `w`.
    ///
    /// Returns [`Error::Overflow`] if the total length exceeds
    /// [`TPKT_MAX_LEN`].
    pub fn encode(&self, w: &mut Writer) -> Result<()> {
        let total = self.encoded_len();
        if total > TPKT_MAX_LEN {
            return Err(Error::Overflow {
                field: "TPKT length",
            });
        }
        w.write_u8(TPKT_VERSION);
        w.write_u8(0x00);
        w.write_u16_be(total as u16);
        w.write_bytes(self.payload);
        Ok(())
    }

    /// Encode into a fresh `Vec`.
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut w = Writer::with_capacity(self.encoded_len());
        self.encode(&mut w)?;
        Ok(w.into_vec())
    }

    /// Decode a single TPKT packet from the front of `buf`.
    ///
    /// The input must contain the complete packet; use
    /// [`Tpkt::peek_total_len`] first when reading from a stream to learn how
    /// many bytes a packet needs.
    pub fn decode(buf: &'a [u8]) -> Result<Tpkt<'a>> {
        let mut r = Reader::new(buf);
        let version = r.read_u8()?;
        if version != TPKT_VERSION {
            return Err(Error::InvalidValue {
                field: "TPKT version",
                value: format!("0x{version:02X}"),
            });
        }
        let _reserved = r.read_u8()?;
        let total = r.read_u16_be()? as usize;
        if total < TPKT_HEADER_LEN {
            return Err(Error::InvalidLength {
                field: "TPKT length",
                length: total,
            });
        }
        let payload_len = total - TPKT_HEADER_LEN;
        let payload = r.read_bytes(payload_len)?;
        Ok(Tpkt { payload })
    }

    /// Peek the declared total length of the packet at the front of `buf`.
    ///
    /// Reads only the 4-byte header, so it works on a partial buffer while
    /// framing a TCP stream. Returns `Ok(None)` when fewer than
    /// [`TPKT_HEADER_LEN`] bytes are available.
    pub fn peek_total_len(buf: &[u8]) -> Result<Option<usize>> {
        if buf.len() < TPKT_HEADER_LEN {
            return Ok(None);
        }
        if buf[0] != TPKT_VERSION {
            return Err(Error::InvalidValue {
                field: "TPKT version",
                value: format!("0x{:02X}", buf[0]),
            });
        }
        let total = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        if total < TPKT_HEADER_LEN {
            return Err(Error::InvalidLength {
                field: "TPKT length",
                length: total,
            });
        }
        Ok(Some(total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let payload = [0xE0, 0x00, 0x00, 0x00, 0x00];
        let bytes = Tpkt::new(&payload).to_vec().unwrap();
        assert_eq!(bytes[0], 0x03);
        assert_eq!(bytes[1], 0x00);
        assert_eq!(&bytes[2..4], &[0x00, 0x09]); // 4 + 5
        let decoded = Tpkt::decode(&bytes).unwrap();
        assert_eq!(decoded.payload, &payload);
    }

    #[test]
    fn rejects_bad_version() {
        let bytes = [0x04, 0x00, 0x00, 0x04];
        assert!(matches!(
            Tpkt::decode(&bytes).unwrap_err(),
            Error::InvalidValue {
                field: "TPKT version",
                ..
            }
        ));
    }

    #[test]
    fn rejects_length_below_header() {
        let bytes = [0x03, 0x00, 0x00, 0x03];
        assert!(matches!(
            Tpkt::decode(&bytes).unwrap_err(),
            Error::InvalidLength {
                field: "TPKT length",
                length: 3
            }
        ));
    }

    #[test]
    fn peek_needs_full_header() {
        assert_eq!(Tpkt::peek_total_len(&[0x03, 0x00]).unwrap(), None);
        assert_eq!(
            Tpkt::peek_total_len(&[0x03, 0x00, 0x00, 0x09]).unwrap(),
            Some(9)
        );
    }

    #[test]
    fn truncated_payload_errors() {
        // Header claims 9 bytes total but only 6 are present.
        let bytes = [0x03, 0x00, 0x00, 0x09, 0xE0, 0x00];
        assert!(matches!(
            Tpkt::decode(&bytes).unwrap_err(),
            Error::UnexpectedEof { .. }
        ));
    }
}
