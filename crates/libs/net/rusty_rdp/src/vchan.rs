//! Static virtual channel chunking (MS-RDPBCGR 2.2.6.1), std-only.
//!
//! Every static virtual channel the client registers (`ClientNetworkData` /
//! `TS_UD_CS_NET` — `"cliprdr"`, `"rdpdr"`, `"DRDYNVC"`, ...) carries its
//! traffic wrapped in an 8-byte `CHANNEL_PDU_HEADER` on top of the usual MCS
//! Send Data Request/Indication. Unlike the main I/O channel's Share
//! Control/Data framing, a single logical message on a virtual channel can be
//! split across several PDUs when it exceeds the negotiated chunk size — this
//! module is the codec for that framing: [`chunk`] splits an outbound message
//! into wire-ready chunks, and [`Reassembler`] puts inbound chunks back
//! together.
//!
//! This is the client-facing (`SEC_ENCRYPT`-wrappable) layer beneath any
//! static-channel protocol — [`crate::dvc`] (which is itself carried over the
//! `"DRDYNVC"` static channel) or a redirection protocol like CLIPRDR.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// Length of the `CHANNEL_PDU_HEADER` in bytes.
pub const CHANNEL_PDU_HEADER_LEN: usize = 8;

/// The default virtual channel chunk size (MS-RDPBCGR 2.2.7.1.10) absent a
/// smaller value negotiated in the Virtual Channel Capability Set.
pub const DEFAULT_CHUNK_SIZE: usize = 1600;

/// This chunk is the first in a fragmented message.
pub const CHANNEL_FLAG_FIRST: u32 = 0x0000_0001;
/// This chunk is the last in a fragmented message.
pub const CHANNEL_FLAG_LAST: u32 = 0x0000_0002;
/// The `CHANNEL_PDU_HEADER` itself must be delivered to the channel endpoint
/// (rather than stripped before dispatch).
pub const CHANNEL_FLAG_SHOW_PROTOCOL: u32 = 0x0000_0010;
/// Server-to-client only: all virtual channel traffic must pause.
pub const CHANNEL_FLAG_SUSPEND: u32 = 0x0000_0020;
/// Server-to-client only: resume previously suspended virtual channel traffic.
pub const CHANNEL_FLAG_RESUME: u32 = 0x0000_0040;

/// A decoded `CHANNEL_PDU_HEADER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelPduHeader {
    /// Total length, in bytes, of the reassembled message (repeated
    /// identically in every chunk of one message).
    pub length: u32,
    /// `CHANNEL_FLAG_*` control bits.
    pub flags: u32,
}

impl ChannelPduHeader {
    /// Encode the 8-byte header.
    pub fn encode(&self, w: &mut Writer) {
        w.write_u32_le(self.length);
        w.write_u32_le(self.flags);
    }

    /// Decode the 8-byte header.
    pub fn decode(r: &mut Reader<'_>) -> Result<ChannelPduHeader> {
        Ok(ChannelPduHeader {
            length: r.read_u32_le()?,
            flags: r.read_u32_le()?,
        })
    }
}

/// Split `data` into one or more `CHANNEL_PDU_HEADER`-prefixed chunks no
/// larger than `chunk_size` bytes of payload each, ready to send one per MCS
/// Send Data Request. `data` must be non-empty.
pub fn chunk(data: &[u8], chunk_size: usize) -> Vec<Vec<u8>> {
    debug_assert!(chunk_size > 0);
    let total_length = data.len() as u32;
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let end = (offset + chunk_size).min(data.len());
        let mut flags = 0u32;
        if offset == 0 {
            flags |= CHANNEL_FLAG_FIRST;
        }
        if end == data.len() {
            flags |= CHANNEL_FLAG_LAST;
        }
        let mut w = Writer::with_capacity(CHANNEL_PDU_HEADER_LEN + (end - offset));
        ChannelPduHeader {
            length: total_length,
            flags,
        }
        .encode(&mut w);
        w.write_bytes(&data[offset..end]);
        out.push(w.into_vec());
        offset = end;
    }
    out
}

/// Reassembles the chunks of one static virtual channel's traffic stream.
///
/// A transport keeps one `Reassembler` per registered virtual channel (they
/// are independent byte streams); feed it each `CHANNEL_PDU_HEADER`-prefixed
/// PDU payload as it arrives on that channel.
#[derive(Debug, Default)]
pub struct Reassembler {
    buf: Vec<u8>,
    expected_len: Option<u32>,
}

impl Reassembler {
    /// Create an empty reassembler.
    pub fn new() -> Self {
        Reassembler {
            buf: Vec::new(),
            expected_len: None,
        }
    }

    /// Feed one chunk (a complete Virtual Channel PDU payload: header +
    /// data). Returns the reassembled message once the `CHANNEL_FLAG_LAST`
    /// chunk arrives, `None` while a message is still in progress.
    pub fn feed(&mut self, pdu: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut r = Reader::new(pdu);
        let header = ChannelPduHeader::decode(&mut r)?;
        let data = r.peek_remaining();

        if header.flags & CHANNEL_FLAG_FIRST != 0 {
            self.buf.clear();
            self.expected_len = Some(header.length);
        }
        let expected = self.expected_len.ok_or(Error::InvalidValue {
            field: "virtual channel chunk",
            value: "middle/last chunk with no preceding FIRST".to_string(),
        })?;
        if header.length != expected {
            return Err(Error::InvalidValue {
                field: "CHANNEL_PDU_HEADER length",
                value: format!("{} (expected {expected})", header.length),
            });
        }
        self.buf.extend_from_slice(data);

        if header.flags & CHANNEL_FLAG_LAST != 0 {
            if self.buf.len() as u32 != expected {
                return Err(Error::InvalidLength {
                    field: "reassembled virtual channel message",
                    length: self.buf.len(),
                });
            }
            self.expected_len = None;
            return Ok(Some(std::mem::take(&mut self.buf)));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_chunk_roundtrip() {
        let data = b"hello, virtual channel";
        let chunks = chunk(data, 1600);
        assert_eq!(chunks.len(), 1);

        let mut re = Reassembler::new();
        assert_eq!(re.feed(&chunks[0]).unwrap(), Some(data.to_vec()));
    }

    #[test]
    fn multi_chunk_spec_example() {
        // The exact MS-RDPBCGR reassembly-page example: 2062 bytes at a
        // 1000-byte chunk size → chunks of 1000, 1000, 62.
        let data = vec![0x5Au8; 2062];
        let chunks = chunk(&data, 1000);
        assert_eq!(chunks.len(), 3);

        let hdr = |c: &[u8]| {
            let mut r = Reader::new(c);
            ChannelPduHeader::decode(&mut r).unwrap()
        };
        assert_eq!(hdr(&chunks[0]).flags, CHANNEL_FLAG_FIRST);
        assert_eq!(hdr(&chunks[0]).length, 2062);
        assert_eq!(chunks[0].len(), CHANNEL_PDU_HEADER_LEN + 1000);
        assert_eq!(hdr(&chunks[1]).flags, 0);
        assert_eq!(chunks[1].len(), CHANNEL_PDU_HEADER_LEN + 1000);
        assert_eq!(hdr(&chunks[2]).flags, CHANNEL_FLAG_LAST);
        assert_eq!(chunks[2].len(), CHANNEL_PDU_HEADER_LEN + 62);

        let mut re = Reassembler::new();
        assert_eq!(re.feed(&chunks[0]).unwrap(), None);
        assert_eq!(re.feed(&chunks[1]).unwrap(), None);
        assert_eq!(re.feed(&chunks[2]).unwrap(), Some(data));
    }

    #[test]
    fn reassembler_handles_consecutive_messages() {
        let mut re = Reassembler::new();
        for msg in [b"first message".to_vec(), b"second one".to_vec()] {
            let chunks = chunk(&msg, 4);
            let mut result = None;
            for c in &chunks {
                result = re.feed(c).unwrap();
            }
            assert_eq!(result, Some(msg));
        }
    }

    #[test]
    fn middle_chunk_without_first_is_rejected() {
        let mut re = Reassembler::new();
        let mut w = Writer::new();
        ChannelPduHeader {
            length: 10,
            flags: CHANNEL_FLAG_LAST,
        }
        .encode(&mut w);
        w.write_bytes(b"0123456789");
        assert!(re.feed(w.as_slice()).is_err());
    }

    #[test]
    fn inconsistent_length_is_rejected() {
        let mut re = Reassembler::new();
        let mut first = Writer::new();
        ChannelPduHeader {
            length: 10,
            flags: CHANNEL_FLAG_FIRST,
        }
        .encode(&mut first);
        first.write_bytes(b"01234");
        re.feed(first.as_slice()).unwrap();

        let mut second = Writer::new();
        ChannelPduHeader {
            length: 999, // inconsistent with the FIRST chunk
            flags: CHANNEL_FLAG_LAST,
        }
        .encode(&mut second);
        second.write_bytes(b"56789");
        assert!(re.feed(second.as_slice()).is_err());
    }
}
