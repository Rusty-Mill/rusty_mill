//! Handles to send a response body stream (mirrors h2::server::SendStream).

use crate::error::{ErrorCode, Result};
use crate::frame;

/// A handle to send the body of a response stream.
#[derive(Debug)]
pub struct SendStream {
    stream_id: u32,
}

impl SendStream {
    pub fn new(stream_id: u32) -> Self {
        SendStream { stream_id }
    }

    /// Send data on this stream.
    pub fn send_data(&mut self, data: Vec<u8>) -> Result<()> {
        let frame = frame::DataFrame::new(self.stream_id, data, false);
        let mut buf = Vec::new();
        frame.encode(&mut buf);
        // In a real async impl, we would queue `buf` for transport write.
        let _ = buf;
        Ok(())
    }

    /// Send a HEADERS frame to signal end of stream.
    pub fn send_end_stream(&mut self) -> Result<()> {
        let headers_frame = frame::HeadersFrame {
            stream_id: self.stream_id,
            end_stream: true,
            end_headers: true,
            priority: None,
            header_block_fragment: Vec::new(),
        };
        let mut buf = Vec::new();
        headers_frame.encode(&mut buf);
        let _ = buf;
        Ok(())
    }
}
