//! Wire-level request sending (HEADERS + DATA frame construction).

use crate::frame;
use crate::hpack;
use crate::hpack::Encoder;
use crate::error::Result;
use crate::frame as h2_frame;
use crate::frame::header::Flags;

/// Request body + frame representation for sending.
pub struct SendRequest {
    stream_id: u32,
}

impl SendRequest {
    /// Create a new send request on the given stream.
    pub fn new(stream_id: u32) -> Self {
        SendRequest { stream_id }
    }

    /// Send a request on this stream: encode headers via HPACK,
    /// build the HEADERS frame, optionally add a DATA frame for body.
    pub fn send(&mut self, method: &str, path: &str, headers: Vec<(&str, &str)>, body: Option<Vec<u8>>) -> Result<Vec<h2_frame::Frame>> {
        let mut encoder = Encoder::new(4096);
        let mut header_fields = Vec::new();

        header_fields.push(hpack::HeaderField::new(":method", method));
        header_fields.push(hpack::HeaderField::new(":scheme", "http"));
        header_fields.push(hpack::HeaderField::new(":authority", "localhost"));
        header_fields.push(hpack::HeaderField::new(":path", path));

        for (name, value) in &headers {
            header_fields.push(hpack::HeaderField::new(name, value));
        }

        let mut header_block = Vec::new();
        encoder.encode(&header_fields, &mut header_block);

        let mut frames = Vec::new();

        let mut flags = Flags::NONE;
        flags |= Flags::END_HEADERS;
        if body.is_none() {
            flags = Flags::END_STREAM | Flags::END_HEADERS;
        }

        let headers_frame = h2_frame::HeadersFrame {
            stream_id: self.stream_id,
            end_stream: body.is_none(),
            end_headers: true,
            priority: None,
            header_block_fragment: header_block,
        };
        frames.push(h2_frame::Frame::Headers(headers_frame));

        if let Some(body_data) = body {
            let data_frame = h2_frame::DataFrame {
                stream_id: self.stream_id,
                data: body_data,
                end_stream: false,
            };
            frames.push(h2_frame::Frame::Data(data_frame));
        }

        Ok(frames)
    }

    pub fn stream_id(&self) -> u32 {
        self.stream_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_headers_frame_with_hpack() {
        let mut req = SendRequest::new(1);
        let frames = req.send("GET", "/", vec![("host", "example.com")], None).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(matches!(frames[0], h2_frame::Frame::Headers(_)));
    }

    #[test]
    fn sends_headers_and_data_frames_with_body() {
        let mut req = SendRequest::new(3);
        let frames = req.send("POST", "/api", vec![("content-type", "application/json")], Some(b"hello".to_vec())).unwrap();
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames[0], h2_frame::Frame::Headers(_)));
        assert!(matches!(frames[1], h2_frame::Frame::Data(_)));
    }
}
