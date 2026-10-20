//! The async connection driver: reads frames off an I/O object, applies them
//! to the sans-IO [`crate::connect::Connection`], and writes what it asks to
//! send. One task per connection owns the socket; application handles talk
//! to it over channels, so a slow reader backpressures its peer through
//! flow control and nothing else.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use rusty_tokio::io::{AsyncRead, AsyncWrite};

use crate::error::{ErrorCode, H2Error};
use crate::hpack::HeaderField;

/// Why a stream or connection operation failed.
#[derive(Debug)]
pub enum Error {
    /// The peer reset the stream.
    Reset(ErrorCode),
    /// The peer closed the connection (GOAWAY) before this could finish.
    GoAway(ErrorCode),
    /// A protocol error on this side or the peer's.
    Protocol(H2Error),
    /// The transport failed.
    Io(io::Error),
    /// The connection is gone (dropped, or finished).
    Closed,
}

/// The connection's task: spawn it, or await it to run the connection to
/// its end. Resolves when the connection closes.
#[must_use = "the connection only makes progress while this is polled"]
pub struct ConnectionDriver {
    _private: (),
}

impl Future for ConnectionDriver {
    type Output = Result<(), Error>;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}

/// One piece of a received body.
#[derive(Debug, PartialEq, Eq)]
pub enum Body {
    /// Message bytes.
    Data(Vec<u8>),
    /// Trailing headers; always the last piece.
    Trailers(Vec<HeaderField>),
}

/// The receiving half of a stream. Capacity is returned to the peer as each
/// piece is taken, so not reading is what applies backpressure.
pub struct RecvStream {
    _private: (),
}

impl RecvStream {
    /// The next piece of the body; `None` once the stream has ended.
    pub async fn next(&mut self) -> Option<Result<Body, Error>> {
        todo!()
    }
}

/// The sending half of a stream.
pub struct SendStream {
    _private: (),
}

impl SendStream {
    /// Sends `data`, waiting for flow-control capacity as needed.
    pub async fn send_data(&mut self, data: Vec<u8>, end_stream: bool) -> Result<(), Error> {
        let _ = (data, end_stream);
        todo!()
    }
    /// Sends trailers, which end the stream.
    pub async fn send_trailers(&mut self, fields: Vec<HeaderField>) -> Result<(), Error> {
        let _ = fields;
        todo!()
    }
    /// Resets the stream with `code`.
    pub fn reset(&mut self, code: ErrorCode) {
        let _ = code;
        todo!()
    }
}

/// A response's headers and body.
pub struct Response {
    /// The first header block.
    pub headers: Vec<HeaderField>,
    /// The body, then trailers; empty if the headers ended the stream.
    pub body: RecvStream,
}

/// Resolves to the response once its headers arrive.
pub struct ResponseFuture {
    _private: (),
}

impl Future for ResponseFuture {
    type Output = Result<Response, Error>;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}

/// The client side of a connection. Cloning shares the connection.
#[derive(Clone)]
pub struct ClientHandle {
    _private: (),
}

impl ClientHandle {
    /// Starts a request; waits while the peer's concurrent-stream limit is
    /// reached. With `end_stream` there is no body.
    pub async fn send_request(
        &self,
        fields: Vec<HeaderField>,
        end_stream: bool,
    ) -> Result<(SendStream, ResponseFuture), Error> {
        let _ = (fields, end_stream);
        todo!()
    }
    /// Sends a PING and waits for its acknowledgement.
    pub async fn ping(&self) -> Result<(), Error> {
        todo!()
    }
}

/// A request accepted by a server.
pub struct Request {
    /// The request's header block.
    pub headers: Vec<HeaderField>,
    /// The request body, then trailers.
    pub body: RecvStream,
}

/// Answers one request.
pub struct Respond {
    _private: (),
}

impl Respond {
    /// Sends the response headers.
    pub async fn send_response(
        self,
        fields: Vec<HeaderField>,
        end_stream: bool,
    ) -> Result<SendStream, Error> {
        let _ = (fields, end_stream);
        todo!()
    }
}

/// The server side of a connection.
pub struct Acceptor {
    _private: (),
}

impl Acceptor {
    /// The next request; `None` once the connection has closed.
    pub async fn accept(&mut self) -> Option<Result<(Request, Respond), Error>> {
        todo!()
    }
    /// Stops accepting: sends GOAWAY, lets in-flight requests finish.
    pub fn graceful_shutdown(&mut self) {
        todo!()
    }
}

/// Runs a client connection over `io`.
pub fn client<T>(
    io: T,
    builder: crate::client::builder::Builder,
) -> (ClientHandle, ConnectionDriver)
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let _ = (io, builder);
    todo!()
}

/// Runs a server connection over `io`.
pub fn server<T>(io: T, builder: crate::server::builder::Builder) -> (Acceptor, ConnectionDriver)
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let _ = (io, builder);
    todo!()
}
