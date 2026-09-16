use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader, FrameType};

/// RST_STREAM frame (RFC 9113 §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RstStreamFrame {
    pub stream_id: u32,
    pub error_code: ErrorCode,
}

impl RstStreamFrame {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let header = FrameHeader::new(4, FrameType::RstStream, Flags::NONE, self.stream_id);
        header.encode(out);
        out.extend_from_slice(&self.error_code.as_u32().to_be_bytes());
    }

    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Self> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "RST_STREAM frame must be associated with a stream",
            ));
        }
        if header.length != 4 || payload.len() != 4 {
            return Err(H2Error::Connection(
                ErrorCode::FrameSizeError,
                "RST_STREAM frame must be exactly 4 octets",
            ));
        }
        let code = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
        Ok(RstStreamFrame {
            stream_id: header.stream_id,
            error_code: ErrorCode::from_u32(code),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = RstStreamFrame {
            stream_id: 9,
            error_code: ErrorCode::Cancel,
        };
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = RstStreamFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }
}
