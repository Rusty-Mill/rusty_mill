use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader, FrameType};
use crate::frame::padding;

/// PUSH_PROMISE frame (RFC 9113 §6.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushPromiseFrame {
    pub stream_id: u32,
    pub end_headers: bool,
    pub promised_stream_id: u32,
    pub header_block_fragment: Vec<u8>,
}

impl PushPromiseFrame {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let flags = if self.end_headers {
            Flags::END_HEADERS
        } else {
            Flags::NONE
        };
        let mut body = Vec::with_capacity(4 + self.header_block_fragment.len());
        body.extend_from_slice(&(self.promised_stream_id & 0x7fff_ffff).to_be_bytes());
        body.extend_from_slice(&self.header_block_fragment);

        let header = FrameHeader::new(
            body.len() as u32,
            FrameType::PushPromise,
            flags,
            self.stream_id,
        );
        header.encode(out);
        out.extend_from_slice(&body);
    }

    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Self> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "PUSH_PROMISE frame must be associated with a stream",
            ));
        }
        if payload.len() != header.length as usize {
            return Err(H2Error::Incomplete);
        }
        let (unpadded, _pad_len) = padding::strip_padding(header, payload)?;
        if unpadded.len() < 4 {
            return Err(H2Error::Connection(
                ErrorCode::FrameSizeError,
                "PUSH_PROMISE frame too short for promised stream id",
            ));
        }
        let promised_stream_id =
            u32::from_be_bytes([unpadded[0], unpadded[1], unpadded[2], unpadded[3]]) & 0x7fff_ffff;
        Ok(PushPromiseFrame {
            stream_id: header.stream_id,
            end_headers: header.flags.contains(Flags::END_HEADERS),
            promised_stream_id,
            header_block_fragment: unpadded[4..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = PushPromiseFrame {
            stream_id: 1,
            end_headers: true,
            promised_stream_id: 2,
            header_block_fragment: vec![1, 2, 3, 4],
        };
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = PushPromiseFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }
}
