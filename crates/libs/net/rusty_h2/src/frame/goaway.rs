use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader, FrameType};

/// GOAWAY frame (RFC 9113 §6.8). Always associated with stream 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoAwayFrame {
    pub last_stream_id: u32,
    pub error_code: ErrorCode,
    pub debug_data: Vec<u8>,
}

impl GoAwayFrame {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let length = 8 + self.debug_data.len() as u32;
        let header = FrameHeader::new(length, FrameType::GoAway, Flags::NONE, 0);
        header.encode(out);
        out.extend_from_slice(&(self.last_stream_id & 0x7fff_ffff).to_be_bytes());
        out.extend_from_slice(&self.error_code.as_u32().to_be_bytes());
        out.extend_from_slice(&self.debug_data);
    }

    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Self> {
        if header.stream_id != 0 {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "GOAWAY frame must be on stream 0",
            ));
        }
        if payload.len() != header.length as usize || payload.len() < 8 {
            return Err(H2Error::Connection(
                ErrorCode::FrameSizeError,
                "GOAWAY frame too short",
            ));
        }
        let last_stream_id =
            u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) & 0x7fff_ffff;
        let error_code = ErrorCode::from_u32(u32::from_be_bytes([
            payload[4], payload[5], payload[6], payload[7],
        ]));
        Ok(GoAwayFrame {
            last_stream_id,
            error_code,
            debug_data: payload[8..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = GoAwayFrame {
            last_stream_id: 17,
            error_code: ErrorCode::EnhanceYourCalm,
            debug_data: b"slow down".to_vec(),
        };
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = GoAwayFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }
}
