use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader, FrameType};

/// PING frame (RFC 9113 §6.7). Always exactly 8 octets of opaque data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingFrame {
    pub ack: bool,
    pub opaque_data: [u8; 8],
}

impl PingFrame {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let flags = if self.ack { Flags::ACK } else { Flags::NONE };
        let header = FrameHeader::new(8, FrameType::Ping, flags, 0);
        header.encode(out);
        out.extend_from_slice(&self.opaque_data);
    }

    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Self> {
        if header.stream_id != 0 {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "PING frame must be on stream 0",
            ));
        }
        if header.length != 8 || payload.len() != 8 {
            return Err(H2Error::Connection(
                ErrorCode::FrameSizeError,
                "PING frame must be exactly 8 octets",
            ));
        }
        let mut opaque_data = [0u8; 8];
        opaque_data.copy_from_slice(payload);
        Ok(PingFrame {
            ack: header.flags.contains(Flags::ACK),
            opaque_data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = PingFrame {
            ack: false,
            opaque_data: [1, 2, 3, 4, 5, 6, 7, 8],
        };
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = PingFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }

    #[test]
    fn wrong_length_rejected() {
        let header = FrameHeader::new(4, FrameType::Ping, Flags::NONE, 0);
        assert!(PingFrame::decode(&header, &[0; 4]).is_err());
    }
}
