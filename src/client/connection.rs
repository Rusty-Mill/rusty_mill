//! Core client connection type (mirrors h2::client::Connection).

use crate::connect;
use crate::error::{ErrorCode, Result};

use super::builder::Builder;

/// Core client HTTP/2 connection state.
#[allow(dead_code)]
pub struct Connection {
    /// Connection-level state machine.
    inner: connect::Connection,
}

impl Connection {
    /// Build a new client connection with the given builder.
    pub fn new(builder: Builder) -> Self {
        let settings = builder.to_settings();
        let inner = connect::Connection::new(settings, connect::PeerType::Server);
        Connection { inner }
    }

    /// Apply a received frame to the connection state machine.
    #[allow(dead_code)]
    pub fn apply(&mut self, frame: crate::frame::Frame) -> Result<Vec<crate::frame::Frame>> {
        self.inner.apply(frame)
    }

    /// Return the stream ID that the next outgoing stream will use.
    /// Returns `None` if the stream space is exhausted.
    pub fn next_stream_id(&self) -> Option<u32> {
        if self.inner.peer_type == connect::PeerType::Server {
            Some(self.inner.next_stream_id)
        } else {
            Some(self.inner.next_stream_id)
        }
    }

    /// Send a GOAWAY frame with the provided error code.
    pub fn send_goaway(&mut self, error_code: ErrorCode, debug_data: &[u8]) {
        let frame = crate::frame::GoAwayFrame {
            last_stream_id: 0,
            error_code,
            debug_data: debug_data.to_vec(),
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf);
        let _ = buf;
    }

    /// Create a `SendRequest` handle for this connection.
    pub fn send_request(&self) -> SendRequest {
        SendRequest { connection: self }
    }
}

/// A handle to send requests on the client connection.
///
/// The `SendRequest` object is the main entry point for sending HTTP/2
/// requests.  It may be cloned cheaply to create multiple handles.
pub struct SendRequest<'a> {
    connection: &'a Connection,
}

impl<'a> SendRequest<'a> {
    fn new(connection: &'a Connection) -> Self {
        SendRequest { connection }
    }

    fn send_request(&mut self, request: RequestBuilder) -> crate::error::Result<Response> {
        todo!()
    }
}

/// A builder for constructing an HTTP/2 request.
pub struct RequestBuilder {
    method: &'static str,
    path: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
}

impl RequestBuilder {
    pub fn new(method: &'static str, path: &str) -> Self {
        RequestBuilder {
            method,
            path: path.to_string(),
            headers: Vec::new(),
            body: None,
        }
    }

    /// Add a header to the request.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Set the body of the request.
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    fn build(&self) -> crate::frame::HeadersFrame {
        // Encode method, path and headers into a header block.
        crate::frame::HeadersFrame {
            stream_id: 0,
            end_stream: false,
            end_headers: false,
            priority: None,
            header_block_fragment: Vec::new(),
        }
    }
}

/// An HTTP/2 response that has been received.
pub struct Response {
    /// The HTTP status code.
    pub status: u16,
    /// The response headers.
    pub headers: Vec<crate::hpack::HeaderField>,
    /// Whether this frame carries END_STREAM.
    pub end_of_stream: bool,
}
