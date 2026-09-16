use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader, FrameType};
use crate::frame::padding;

/// Stream dependency + weight, present when the PRIORITY flag is set on a
/// HEADERS frame, or always on a PRIORITY frame (RFC 9113 §5.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Priority {
    pub exclusive: bool,
    pub dependency: u32, // 31-bit stream id
    pub weight: u8,      // encoded as weight-1 on the wire; this is the real 1-256 weight
}

impl Priority {
    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < 5 {
            return Err(H2Error::Incomplete);
        }
        let raw = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        Ok(Priority {
            exclusive: raw & 0x8000_0000 != 0,
            dependency: raw & 0x7fff_ffff,
            weight: buf[4].wrapping_add(1),
        })
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        let mut raw = self.dependency & 0x7fff_ffff;
        if self.exclusive {
            raw |= 0x8000_0000;
        }
        out.extend_from_slice(&raw.to_be_bytes());
        out.push(self.weight.wrapping_sub(1));
    }
}

/// HEADERS frame (RFC 9113 §6.2). `header_block_fragment` is the raw,
/// still-HPACK-encoded byte string; combine with any CONTINUATION frames
/// before running it through the HPACK decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadersFrame {
    pub stream_id: u32,
    pub end_stream: bool,
    pub end_headers: bool,
    pub priority: Option<Priority>,
    pub header_block_fragment: Vec<u8>,
}

impl HeadersFrame {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let mut flags = Flags::NONE;
        if self.end_stream {
            flags |= Flags::END_STREAM;
        }
        if self.end_headers {
            flags |= Flags::END_HEADERS;
        }
        if self.priority.is_some() {
            flags |= Flags::PRIORITY;
        }

        let mut body = Vec::new();
        if let Some(p) = &self.priority {
            p.encode(&mut body);
        }
        body.extend_from_slice(&self.header_block_fragment);

        let header = FrameHeader::new(body.len() as u32, FrameType::Headers, flags, self.stream_id);
        header.encode(out);
        out.extend_from_slice(&body);
    }

    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Self> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "HEADERS frame must be associated with a stream",
            ));
        }
        if payload.len() != header.length as usize {
            return Err(H2Error::Incomplete);
        }
        let (unpadded, _pad_len) = padding::strip_padding(header, payload)?;

        let mut rest = unpadded;
        let priority = if header.flags.contains(Flags::PRIORITY) {
            let p = Priority::decode(rest)?;
            rest = &rest[5..];
            Some(p)
        } else {
            None
        };

        Ok(HeadersFrame {
            stream_id: header.stream_id,
            end_stream: header.flags.contains(Flags::END_STREAM),
            end_headers: header.flags.contains(Flags::END_HEADERS),
            priority,
            header_block_fragment: rest.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_without_priority() {
        let f = HeadersFrame {
            stream_id: 1,
            end_stream: true,
            end_headers: true,
            priority: None,
            header_block_fragment: vec![1, 2, 3],
        };
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = HeadersFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }

    #[test]
    fn roundtrip_with_priority() {
        let f = HeadersFrame {
            stream_id: 3,
            end_stream: false,
            end_headers: true,
            priority: Some(Priority {
                exclusive: true,
                dependency: 1,
                weight: 200,
            }),
            header_block_fragment: vec![9, 9, 9],
        };
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = HeadersFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }
}
