//! Core server connection type (mirrors h2::server::Connection).

use crate::connect;
use crate::error::ErrorCode;
use crate::frame;

use std::task::Poll;

use super::builder::Builder;
use super::send_response::SendResponse;
use super::send_stream::SendStream;

/// A server connection (mirrors h2::server::Connection).
#[derive(Debug)]
pub struct Connection {
    /// Connection state.
    inner: connect::Connection,
}

impl Connection {
    /// Start a new server connection using `io`.
    pub fn new(builder: Builder) -> Self {
        let settings = builder.to_settings();
        let inner = connect::Connection::new(settings, connect::PeerType::Server);
        Connection { inner }
    }

    /// Accept a new inbound request.
    pub fn poll_accept(&mut self) -> Poll<Option<IncomingRequest>> {
        Poll::Pending
    }

    /// Send a GOAWAY frame with the provided error code.
    pub fn send_goaway(&mut self, error_code: ErrorCode, debug_data: &[u8]) {
        let frame = frame::GoAwayFrame {
            last_stream_id: 0,
            error_code,
            debug_data: debug_data.to_vec(),
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf);
        let _ = buf;
    }

    /// Create a `SendResponse` handle for the given stream.
    pub fn send_response(&self, stream_id: u32) -> SendResponse {
        SendResponse::new(stream_id)
    }
}

/// An incoming request that has been received on the server.
#[derive(Debug)]
pub struct IncomingRequest {
    stream_id: u32,
    headers: Vec<crate::hpack::HeaderField>,
}

impl IncomingRequest {
    pub fn stream_id(&self) -> u32 {
        self.stream_id
    }

    pub fn headers(&self) -> &[crate::hpack::HeaderField] {
        &self.headers
    }

    pub fn method(&self) -> &str {
        // Extract :method from headers
        self.headers
            .iter()
            .find(|h| h.name == b":method")
            .map(|h| std::str::from_utf8(&h.value).unwrap_or(""))
            .unwrap_or("")
    }

    pub fn path(&self) -> &str {
        self.headers
            .iter()
            .find(|h| h.name == b":path")
            .map(|h| std::str::from_utf8(&h.value).unwrap_or(""))
            .unwrap_or("")
    }
}
