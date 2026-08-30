//! Handles to send a response (mirrors h2::server::SendResponse).

use crate::error::{ErrorCode, Result};
use crate::frame;
use crate::hpack;

/// A handle to send a response to a client request.
#[derive(Debug)]
pub struct SendResponse {
    stream_id: u32,
    send_stream: SendStream,
}

impl SendResponse {
    /// Create a new SendResponse handle.
    pub fn new(stream_id: u32) -> Self {
        SendResponse {
            stream_id,
            send_stream: SendStream::new(stream_id),
        }
    }

    /// Encode response headers carrying `status` into a HEADERS frame,
    /// returning it for the caller's transport to write to the wire,
    /// along with a [`SendStream`] handle for the response body.
    pub fn send_response(&mut self, status: u16) -> Result<(frame::Frame, SendStream)> {
        let mut encoder = hpack::Encoder::new(hpack::DEFAULT_HEADER_TABLE_SIZE);
        let mut header_block = Vec::new();
        encoder.encode(
            &[hpack::HeaderField::new(":status", status.to_string())],
            &mut header_block,
        );

        let headers_frame = frame::HeadersFrame {
            stream_id: self.stream_id,
            end_stream: false,
            end_headers: true,
            priority: None,
            header_block_fragment: header_block,
        };

        Ok((
            frame::Frame::Headers(headers_frame),
            self.send_stream.clone(),
        ))
    }

    /// Set the maximum frame size for this stream.
    pub fn set_max_frame_size(&mut self, size: u32) {
        let _ = size;
    }
}

/// A handle to send the body (stream data) of a response.
#[derive(Debug, Clone)]
pub struct SendStream {
    stream_id: u32,
}

impl SendStream {
    pub fn new(stream_id: u32) -> Self {
        SendStream { stream_id }
    }

    /// Build a DATA frame carrying `data`, for the caller's transport to
    /// write and apply to the connection.
    pub fn send_data(&mut self, data: Vec<u8>) -> frame::Frame {
        frame::Frame::Data(frame::DataFrame::new(self.stream_id, data, false))
    }

    /// Build a DATA frame carrying `data` with `END_STREAM` set.
    pub fn send_data_eos(&mut self, data: Vec<u8>) -> frame::Frame {
        frame::Frame::Data(frame::DataFrame::new(self.stream_id, data, true))
    }

    /// Build an RST_STREAM frame resetting this stream with `code`.
    pub fn reset(&mut self, code: ErrorCode) -> frame::Frame {
        frame::Frame::RstStream(frame::RstStreamFrame {
            stream_id: self.stream_id,
            error_code: code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_response_encodes_the_status_into_a_headers_frame() {
        let mut resp = SendResponse::new(1);
        let (frame, _stream) = resp.send_response(200).unwrap();
        let frame::Frame::Headers(h) = &frame else {
            panic!("expected HEADERS, got {frame:?}")
        };
        assert!(!h.header_block_fragment.is_empty());

        let mut decoder = hpack::Decoder::new(hpack::DEFAULT_HEADER_TABLE_SIZE);
        let fields = decoder.decode(&h.header_block_fragment).unwrap();
        assert_eq!(fields[0].name, b":status");
        assert_eq!(fields[0].value, b"200");
    }

    #[test]
    fn send_stream_builds_data_and_rst_stream_frames() {
        let mut stream = SendStream::new(1);
        assert!(matches!(stream.send_data(b"hi".to_vec()), frame::Frame::Data(d) if !d.end_stream));
        assert!(
            matches!(stream.send_data_eos(b"bye".to_vec()), frame::Frame::Data(d) if d.end_stream)
        );
        assert!(matches!(
            stream.reset(ErrorCode::Cancel),
            frame::Frame::RstStream(_)
        ));
    }
}
