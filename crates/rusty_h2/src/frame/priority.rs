use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader, FrameType};
use crate::frame::headers::Priority;

/// PRIORITY frame (RFC 9113 §6.3). Deprecated by RFC 9113 but still a valid
/// frame type that implementations must be able to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityFrame {
    pub stream_id: u32,
    pub priority: Priority,
}

impl PriorityFrame {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let header = FrameHeader::new(5, FrameType::Priority, Flags::NONE, self.stream_id);
        header.encode(out);
        self.priority.encode(out);
    }

    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Self> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "PRIORITY frame must be associated with a stream",
            ));
        }
        if header.length != 5 || payload.len() != 5 {
            return Err(H2Error::Stream(
                header.stream_id,
                ErrorCode::FrameSizeError,
                "PRIORITY frame must be exactly 5 octets",
            ));
        }
        Ok(PriorityFrame {
            stream_id: header.stream_id,
            priority: Priority::decode(payload)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = PriorityFrame {
            stream_id: 5,
            priority: Priority {
                exclusive: false,
                dependency: 3,
                weight: 16,
            },
        };
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = PriorityFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }

    #[test]
    fn wrong_size_is_stream_error() {
        let header = FrameHeader::new(4, FrameType::Priority, Flags::NONE, 1);
        assert!(PriorityFrame::decode(&header, &[0; 4]).is_err());
    }
}
