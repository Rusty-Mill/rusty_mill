use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader, FrameType};

/// CONTINUATION frame (RFC 9113 §6.10). Carries the overflow of a header
/// block that didn't fit in the preceding HEADERS/PUSH_PROMISE frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationFrame {
    pub stream_id: u32,
    pub end_headers: bool,
    pub header_block_fragment: Vec<u8>,
}

impl ContinuationFrame {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let flags = if self.end_headers {
            Flags::END_HEADERS
        } else {
            Flags::NONE
        };
        let header = FrameHeader::new(
            self.header_block_fragment.len() as u32,
            FrameType::Continuation,
            flags,
            self.stream_id,
        );
        header.encode(out);
        out.extend_from_slice(&self.header_block_fragment);
    }

    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Self> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "CONTINUATION frame must be associated with a stream",
            ));
        }
        if payload.len() != header.length as usize {
            return Err(H2Error::Incomplete);
        }
        Ok(ContinuationFrame {
            stream_id: header.stream_id,
            end_headers: header.flags.contains(Flags::END_HEADERS),
            header_block_fragment: payload.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = ContinuationFrame {
            stream_id: 1,
            end_headers: true,
            header_block_fragment: vec![1, 2, 3],
        };
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = ContinuationFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }
}
