use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader, FrameType};

/// DATA frame (RFC 9113 §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataFrame {
    pub stream_id: u32,
    pub end_stream: bool,
    pub data: Vec<u8>,
}

impl DataFrame {
    pub fn new(stream_id: u32, data: Vec<u8>, end_stream: bool) -> Self {
        DataFrame {
            stream_id,
            end_stream,
            data,
        }
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        let flags = if self.end_stream {
            Flags::END_STREAM
        } else {
            Flags::NONE
        };
        let header = FrameHeader::new(
            self.data.len() as u32,
            FrameType::Data,
            flags,
            self.stream_id,
        );
        header.encode(out);
        out.extend_from_slice(&self.data);
    }

    /// Decode a DATA frame body. `header.length` bytes must be present in `payload`.
    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Self> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "DATA frame must be associated with a stream",
            ));
        }
        if payload.len() != header.length as usize {
            return Err(H2Error::Incomplete);
        }
        let (data, _pad_len) = super::padding::strip_padding(header, payload)?;
        Ok(DataFrame {
            stream_id: header.stream_id,
            end_stream: header.flags.contains(Flags::END_STREAM),
            data: data.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = DataFrame::new(3, b"hello".to_vec(), true);
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = DataFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }

    #[test]
    fn stream_zero_is_protocol_error() {
        let header = FrameHeader::new(0, FrameType::Data, Flags::NONE, 0);
        assert!(DataFrame::decode(&header, &[]).is_err());
    }
}
