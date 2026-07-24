/// Stub for send request.
use crate::frame;
use crate::error::Result;

/// A handle to send requests.
pub struct SendRequest {
    stream_id: u32,
    connection: Option<crate::connect::Connection>,
}

impl SendRequest {
    /// Create a new send request handle.
    pub fn new(stream_id: u32, connection: crate::connect::Connection) -> Self {
        SendRequest {
            stream_id,
            connection: Some(connection),
        }
    }

    /// Send a request.
    pub fn send(&mut self, request: crate::request::Request) -> Result<crate::response::Response> {
        todo!()
    }

    pub fn stream_id(&self) -> u32 {
        self.stream_id
    }
}
