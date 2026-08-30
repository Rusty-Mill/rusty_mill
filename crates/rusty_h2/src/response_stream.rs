/// Stub for response stream.
use crate::frame;
use crate::error::Result;

/// A stream for sending a response.
pub struct ResponseStream {
    stream_id: u32,
    body: Vec<u8>,
    end_of_stream: bool,
}

impl ResponseStream {
    /// Create a new response stream.
    pub fn new(stream_id: u32) -> Self {
        ResponseStream {
            stream_id,
            body: Vec::new(),
            end_of_stream: false,
        }
    }

    /// Add data to the response body.
    pub fn append_data(&mut self, data: Vec<u8>) {
        self.body.extend(data);
    }

    /// Signal end of stream.
    pub fn end_stream(&mut self) {
        self.end_of_stream = true;
    }

    /// Encode this stream into a frame for sending.
    pub fn encode(&self) -> Result<frame::Frame> {
        Ok(frame::Frame::Data(frame::DataFrame {
            stream_id: self.stream_id,
            data: self.body.clone(),
            end_stream: self.end_of_stream,
        }))
    }

    pub fn stream_id(&self) -> u32 {
        self.stream_id
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn end_of_stream(&self) -> bool {
        self.end_of_stream
    }
}
