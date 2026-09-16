use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader, FrameType};

/// WINDOW_UPDATE frame (RFC 9113 §6.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowUpdateFrame {
    pub stream_id: u32,
    pub window_size_increment: u32, // 31-bit, 1..=0x7fffffff
}

impl WindowUpdateFrame {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let header = FrameHeader::new(4, FrameType::WindowUpdate, Flags::NONE, self.stream_id);
        header.encode(out);
        out.extend_from_slice(&(self.window_size_increment & 0x7fff_ffff).to_be_bytes());
    }

    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Self> {
        if header.length != 4 || payload.len() != 4 {
            return Err(H2Error::Connection(
                ErrorCode::FrameSizeError,
                "WINDOW_UPDATE frame must be exactly 4 octets",
            ));
        }
        let increment =
            u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) & 0x7fff_ffff;
        if increment == 0 {
            let code = ErrorCode::ProtocolError;
            return if header.stream_id == 0 {
                Err(H2Error::Connection(
                    code,
                    "WINDOW_UPDATE increment must not be 0",
                ))
            } else {
                Err(H2Error::Stream(
                    header.stream_id,
                    code,
                    "WINDOW_UPDATE increment must not be 0",
                ))
            };
        }
        Ok(WindowUpdateFrame {
            stream_id: header.stream_id,
            window_size_increment: increment,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = WindowUpdateFrame {
            stream_id: 0,
            window_size_increment: 65535,
        };
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = WindowUpdateFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }

    #[test]
    fn zero_increment_rejected() {
        let header = FrameHeader::new(4, FrameType::WindowUpdate, Flags::NONE, 1);
        assert!(WindowUpdateFrame::decode(&header, &[0, 0, 0, 0]).is_err());
    }
}
