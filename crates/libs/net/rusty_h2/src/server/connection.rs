//! Core server connection type (mirrors h2::server::Connection).

use crate::connect;
use crate::error::ErrorCode;
use crate::frame;

use std::collections::VecDeque;
use std::task::Poll;

use super::builder::Builder;
use super::send_response::SendResponse;

/// A server connection (mirrors h2::server::Connection).
#[derive(Debug)]
pub struct Connection {
    /// Connection state.
    inner: connect::Connection,
    /// Requests whose HEADERS have arrived and not yet been accepted.
    accepted: VecDeque<IncomingRequest>,
    /// Everything else the peer sent (body, trailers, resets), in order.
    inbound: Vec<connect::Inbound>,
}

impl Connection {
    /// Start a new server connection using `io`.
    pub fn new(builder: Builder) -> Self {
        let settings = builder.to_settings();
        let inner = connect::Connection::new(settings, connect::PeerType::Server);
        Connection {
            inner,
            accepted: VecDeque::new(),
            inbound: Vec::new(),
        }
    }

    /// Apply a received frame to the connection state machine.
    pub fn apply(&mut self, frame: frame::Frame) -> crate::error::Result<Vec<frame::Frame>> {
        let result = self.inner.apply(frame);
        for event in self.inner.take_inbound() {
            match event {
                connect::Inbound::Headers {
                    stream_id, fields, ..
                } => self.accepted.push_back(IncomingRequest {
                    stream_id,
                    headers: fields,
                }),
                other => self.inbound.push(other),
            }
        }
        result
    }

    /// Drains the body bytes, trailers and resets received since the last
    /// call. A request's own HEADERS come from [`Self::poll_accept`].
    pub fn take_inbound(&mut self) -> Vec<connect::Inbound> {
        std::mem::take(&mut self.inbound)
    }

    /// The connection state machine, for sending headers, data and trailers
    /// and returning flow-control capacity.
    pub fn state_mut(&mut self) -> &mut connect::Connection {
        &mut self.inner
    }

    /// Accept the next request whose HEADERS [`Self::apply`] has seen;
    /// pending when there is none.
    pub fn poll_accept(&mut self) -> Poll<Option<IncomingRequest>> {
        match self.accepted.pop_front() {
            Some(request) => Poll::Ready(Some(request)),
            None => Poll::Pending,
        }
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
