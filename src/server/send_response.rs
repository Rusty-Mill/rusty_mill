//! Handles to send a response (mirrors h2::server::SendResponse).

use crate::error::{ErrorCode, H2Error, Result};
use crate::frame;

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

    /// Send response headers.
    pub fn send_response(&mut self, status: u16) -> Result<SendStream> {
        // Encode a HEADERS frame with the status code and end_headers flag.
        let mut flags = frame::header::Flags::NONE;
        flags |= frame::header::Flags::END_HEADERS;

        let header =
            frame::header::FrameHeader::new(0, frame::FrameType::Headers, flags, self.stream_id);

        // We would construct a proper `HeadersFrame` here with the status code
        // and any headers.  For now we just acknowledge.
        let mut buf = Vec::new();
        header.encode(&mut buf);

        Ok(self.send_stream.clone())
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

    /// Write body data.
    pub fn send_data(&mut self, data: Vec<u8>) {
        let frame = frame::DataFrame::new(self.stream_id, data, false);
        let mut buf = Vec::new();
        frame.encode(&mut buf);
        // Queue `buf` for transport-layer write.
        let _ = buf;
    }

    /// Write body data and signal end of stream.
    pub fn send_data_eos(&mut self, data: Vec<u8>) {
        let frame = frame::DataFrame::new(self.stream_id, data, true);
        let mut buf = Vec::new();
        frame.encode(&mut buf);
        // Queue `buf` for transport-layer write.
        let _ = buf;
    }

    /// Reset the response stream with an error.
    pub fn reset(&mut self, code: ErrorCode) {
        let frame = frame::RstStreamFrame {
            stream_id: self.stream_id,
            error_code: code,
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf);
        let _ = buf;
    }
}
