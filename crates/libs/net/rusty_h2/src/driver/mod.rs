//! The async connection driver: reads frames off an I/O object, applies them
//! to the sans-IO [`crate::connect::Connection`], and writes what it asks to
//! send. One task per connection owns the socket; application handles talk
//! to it through shared state, so a slow reader backpressures its peer
//! through flow control and nothing else.

mod io;
mod shared;

use std::future::{poll_fn, Future};
use std::io as stdio;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use rusty_tokio::io::{AsyncRead, AsyncWrite};

use crate::connect::{Connection, PeerType, ServerSettings};
use crate::error::{ErrorCode, H2Error};
use crate::frame::{Frame, PingFrame};
use crate::hpack::HeaderField;
use shared::{lock, register, Handle, Item, Shared, OUT_HIGH};

pub use io::ConnectionDriver;

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
    Io(stdio::Error),
    /// The connection is gone (dropped, or finished).
    Closed,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Reset(c) => write!(f, "stream reset: {c}"),
            Error::GoAway(c) => write!(f, "connection going away: {c}"),
            Error::Protocol(e) => write!(f, "{e}"),
            Error::Io(e) => write!(f, "{e}"),
            Error::Closed => write!(f, "connection closed"),
        }
    }
}

impl std::error::Error for Error {}

/// One piece of a received body.
#[derive(Debug, PartialEq, Eq)]
pub enum Body {
    /// Message bytes.
    Data(Vec<u8>),
    /// Trailing headers; always the last piece.
    Trailers(Vec<HeaderField>),
}

fn protocol(e: H2Error) -> Error {
    Error::Protocol(e)
}

/// The receiving half of a stream. Capacity is returned to the peer as each
/// piece is taken, so not reading is what applies backpressure.
pub struct RecvStream {
    shared: Handle,
    id: u32,
}

impl RecvStream {
    /// The next piece of the body; `None` once the stream has ended.
    pub async fn next(&mut self) -> Option<Result<Body, Error>> {
        poll_fn(|cx| {
            let mut sh = lock(&self.shared);
            let Some(s) = sh.streams.get_mut(&self.id) else {
                return Poll::Ready(None);
            };
            let Some(item) = s.inbox.pop_front() else {
                if s.remote_closed {
                    return Poll::Ready(None);
                }
                s.recv_waker = Some(cx.waker().clone());
                return Poll::Pending;
            };
            let out = match item {
                Item::Body(Body::Data(d)) => {
                    let n = d.len();
                    sh.release(self.id, n);
                    Some(Ok(Body::Data(d)))
                }
                Item::Body(b) => Some(Ok(b)),
                Item::Fail(f) => Some(Err(f.error())),
            };
            sh.settle(self.id);
            Poll::Ready(out)
        })
        .await
    }
}

impl Drop for RecvStream {
    fn drop(&mut self) {
        let mut sh = lock(&self.shared);
        if let Some(s) = sh.streams.get_mut(&self.id) {
            s.recv_dropped = true;
        }
        sh.discard(self.id);
        sh.settle(self.id);
    }
}

/// The sending half of a stream.
pub struct SendStream {
    shared: Handle,
    id: u32,
    ended: bool,
}

impl SendStream {
    fn check(sh: &Shared, id: u32) -> Result<(), Error> {
        match sh.streams.get(&id) {
            None => Err(Error::Closed),
            Some(s) => match s.abort {
                Some(f) => Err(f.error()),
                None if s.local_closed => Err(Error::Closed),
                None => Ok(()),
            },
        }
    }

    /// Sends `data`, waiting for flow-control capacity as needed.
    pub async fn send_data(&mut self, data: Vec<u8>, end_stream: bool) -> Result<(), Error> {
        let mut off = 0;
        loop {
            let done = poll_fn(|cx| {
                let mut sh = lock(&self.shared);
                if let Err(e) = Self::check(&sh, self.id) {
                    return Poll::Ready(Err(e));
                }
                if sh.backlog() >= OUT_HIGH {
                    register(&mut sh.waiters, cx.waker());
                    return Poll::Pending;
                }
                let cap = sh.conn.send_capacity(self.id);
                if cap == 0 && off < data.len() {
                    register(&mut sh.waiters, cx.waker());
                    return Poll::Pending;
                }
                let n = cap.min(data.len() - off);
                let last = off + n == data.len();
                match sh
                    .conn
                    .send_data(self.id, &data[off..off + n], end_stream && last)
                {
                    Ok(frames) => sh.queue(frames),
                    Err(e) => return Poll::Ready(Err(protocol(e))),
                }
                off += n;
                if last && end_stream {
                    sh.close_local(self.id);
                }
                Poll::Ready(Ok(last))
            })
            .await?;
            if done {
                self.ended |= end_stream;
                return Ok(());
            }
        }
    }

    /// Sends trailers, which end the stream.
    pub async fn send_trailers(&mut self, fields: Vec<HeaderField>) -> Result<(), Error> {
        let mut sh = lock(&self.shared);
        Self::check(&sh, self.id)?;
        let frame = sh.conn.send_trailers(self.id, &fields).map_err(protocol)?;
        sh.queue([frame]);
        sh.close_local(self.id);
        self.ended = true;
        Ok(())
    }

    /// Resets the stream with `code`.
    pub fn reset(&mut self, code: ErrorCode) {
        let mut sh = lock(&self.shared);
        if sh.streams.get(&self.id).is_some_and(|s| s.abort.is_none()) {
            sh.reset(self.id, code);
        }
        self.ended = true;
    }
}

impl Drop for SendStream {
    fn drop(&mut self) {
        if !self.ended {
            self.reset(ErrorCode::Cancel);
        }
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
    shared: Handle,
    id: u32,
    taken: bool,
}

impl Future for ResponseFuture {
    type Output = Result<Response, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let id = self.id;
        let shared = self.shared.clone();
        let mut sh = lock(&shared);
        let Some(s) = sh.streams.get_mut(&id) else {
            return Poll::Ready(Err(Error::Closed));
        };
        if let Some(headers) = s.headers.take() {
            s.headers_taken = true;
            self.taken = true;
            return Poll::Ready(Ok(Response {
                headers,
                body: RecvStream {
                    shared: shared.clone(),
                    id,
                },
            }));
        }
        if let Some(Item::Fail(f)) = s.inbox.front() {
            let e = f.error();
            s.inbox.clear();
            s.headers_taken = true;
            self.taken = true;
            sh.settle(id);
            return Poll::Ready(Err(e));
        }
        s.headers_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for ResponseFuture {
    fn drop(&mut self) {
        if self.taken {
            return;
        }
        let mut sh = lock(&self.shared);
        if let Some(s) = sh.streams.get_mut(&self.id) {
            s.headers_taken = true;
            s.recv_dropped = true;
        }
        sh.discard(self.id);
        sh.settle(self.id);
    }
}

/// Tells the driver when the last [`ClientHandle`] is gone.
struct Token(Handle);

impl Drop for Token {
    fn drop(&mut self) {
        let mut sh = lock(&self.0);
        sh.closing = true;
        sh.wake_driver();
    }
}

/// The client side of a connection. Cloning shares the connection; the
/// connection winds down once every handle is dropped and its calls finish.
#[derive(Clone)]
pub struct ClientHandle {
    shared: Handle,
    _token: Arc<Token>,
}

impl ClientHandle {
    /// Starts a request; waits while the peer's concurrent-stream limit is
    /// reached. With `end_stream` there is no body.
    pub async fn send_request(
        &self,
        fields: Vec<HeaderField>,
        end_stream: bool,
    ) -> Result<(SendStream, ResponseFuture), Error> {
        let id = poll_fn(|cx| {
            let mut sh = lock(&self.shared);
            if sh.closed {
                return Poll::Ready(Err(Error::Closed));
            }
            if let Some((_, code)) = sh.conn.goaway_received() {
                return Poll::Ready(Err(Error::GoAway(code)));
            }
            let full = sh
                .conn
                .remote_settings
                .max_concurrent_streams
                .is_some_and(|max| sh.active >= max as usize);
            if !sh.settings_received || full {
                register(&mut sh.waiters, cx.waker());
                return Poll::Pending;
            }
            let (id, frame) = match sh.conn.open_stream(&fields, end_stream) {
                Ok(v) => v,
                Err(e) => return Poll::Ready(Err(protocol(e))),
            };
            sh.queue([frame]);
            sh.add_stream(id, end_stream);
            Poll::Ready(Ok(id))
        })
        .await?;
        let send = SendStream {
            shared: self.shared.clone(),
            id,
            ended: end_stream,
        };
        let resp = ResponseFuture {
            shared: self.shared.clone(),
            id,
            taken: false,
        };
        Ok((send, resp))
    }

    /// Sends a PING and waits for its acknowledgement.
    pub async fn ping(&self) -> Result<(), Error> {
        let key = {
            let mut sh = lock(&self.shared);
            if sh.closed {
                return Err(Error::Closed);
            }
            let key = sh.next_ping;
            sh.next_ping += 1;
            sh.pings.insert(
                key,
                shared::Ping {
                    acked: false,
                    waker: None,
                },
            );
            sh.queue([Frame::Ping(PingFrame {
                ack: false,
                opaque_data: key.to_be_bytes(),
            })]);
            key
        };
        poll_fn(|cx| {
            let mut sh = lock(&self.shared);
            let acked = sh.pings.get(&key).is_some_and(|p| p.acked);
            if acked {
                sh.pings.remove(&key);
                return Poll::Ready(Ok(()));
            }
            if sh.closed {
                sh.pings.remove(&key);
                return Poll::Ready(Err(Error::Closed));
            }
            if let Some(p) = sh.pings.get_mut(&key) {
                p.waker = Some(cx.waker().clone());
            }
            Poll::Pending
        })
        .await
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
    shared: Handle,
    id: u32,
    used: bool,
}

impl Respond {
    /// Sends the response headers.
    pub async fn send_response(
        mut self,
        fields: Vec<HeaderField>,
        end_stream: bool,
    ) -> Result<SendStream, Error> {
        let mut sh = lock(&self.shared);
        // On failure `self` is dropped unused, which resets the stream and
        // so tells the peer; only a sent response disarms that.
        SendStream::check(&sh, self.id)?;
        let frame = sh
            .conn
            .send_headers(self.id, &fields, end_stream)
            .map_err(protocol)?;
        sh.queue([frame]);
        if end_stream {
            sh.close_local(self.id);
        }
        drop(sh);
        self.used = true;
        Ok(SendStream {
            shared: self.shared.clone(),
            id: self.id,
            ended: end_stream,
        })
    }
}

impl Drop for Respond {
    fn drop(&mut self) {
        if !self.used {
            let mut sh = lock(&self.shared);
            if sh.streams.get(&self.id).is_some_and(|s| s.abort.is_none()) {
                sh.reset(self.id, ErrorCode::InternalError);
            }
        }
    }
}

/// The server side of a connection.
pub struct Acceptor {
    shared: Handle,
}

impl Acceptor {
    /// The next request; `None` once the connection has closed.
    pub async fn accept(&mut self) -> Option<Result<(Request, Respond), Error>> {
        poll_fn(|cx| {
            let mut sh = lock(&self.shared);
            while let Some(id) = sh.accept_queue.pop_front() {
                let Some(s) = sh.streams.get_mut(&id) else {
                    continue;
                };
                let Some(headers) = s.headers.take() else {
                    continue;
                };
                s.headers_taken = true;
                let shared = self.shared.clone();
                return Poll::Ready(Some(Ok((
                    Request {
                        headers,
                        body: RecvStream {
                            shared: shared.clone(),
                            id,
                        },
                    },
                    Respond {
                        shared,
                        id,
                        used: false,
                    },
                ))));
            }
            if sh.closed || sh.closing || sh.conn.goaway_received().is_some() {
                return Poll::Ready(None);
            }
            sh.accept_waker = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    /// Stops accepting: sends GOAWAY, lets in-flight requests finish.
    pub fn graceful_shutdown(&mut self) {
        let mut sh = lock(&self.shared);
        sh.closing = true;
        sh.send_goaway(ErrorCode::NoError);
        sh.wake_driver();
    }
}

impl Drop for Acceptor {
    fn drop(&mut self) {
        self.graceful_shutdown();
        // Nobody will accept what is still queued: refuse it, so the
        // connection can finish.
        let mut sh = lock(&self.shared);
        let queued: Vec<u32> = sh.accept_queue.drain(..).collect();
        for id in queued {
            sh.reset(id, ErrorCode::RefusedStream);
        }
    }
}

fn driver<T>(io: T, shared: Handle, server: bool, max_frame: u32) -> ConnectionDriver<T> {
    ConnectionDriver {
        shared,
        io,
        rbuf: Vec::new(),
        rpos: 0,
        preface_seen: 0,
        preface_needed: server,
        eof: false,
        finishing: None,
        max_frame,
        stalled: false,
        needs_flush: false,
    }
}

fn start(settings: ServerSettings, peer: PeerType) -> (Handle, u32) {
    let server = peer == PeerType::Server;
    let max_frame = settings.max_frame_size;
    let conn = Connection::new(settings, peer);
    (
        Arc::new(Mutex::new(Shared::new(conn, server, !server))),
        max_frame,
    )
}

/// Runs a client connection over `io`.
pub fn client<T>(
    io: T,
    builder: crate::client::builder::Builder,
) -> (ClientHandle, ConnectionDriver<T>)
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut settings = builder.to_settings();
    // Pushed responses are not delivered, so none are invited.
    settings.enable_push = false;
    let (shared, max_frame) = start(settings, PeerType::Client);
    let handle = ClientHandle {
        shared: shared.clone(),
        _token: Arc::new(Token(shared.clone())),
    };
    (handle, driver(io, shared, false, max_frame))
}

/// Runs a server connection over `io`.
pub fn server<T>(io: T, builder: crate::server::builder::Builder) -> (Acceptor, ConnectionDriver<T>)
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (shared, max_frame) = start(builder.to_settings(), PeerType::Server);
    (
        Acceptor {
            shared: shared.clone(),
        },
        driver(io, shared, true, max_frame),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Wake, Waker};

    /// What a waker saw when it was destroyed.
    #[derive(Default)]
    struct Flags {
        dropped: AtomicBool,
        lock_was_free: AtomicBool,
    }

    /// A waker whose destruction checks that the shared mutex is free: a real
    /// waker can own a handle whose `Drop` locks it.
    struct Probe {
        shared: Handle,
        flags: Arc<Flags>,
    }

    impl Wake for Probe {
        fn wake(self: Arc<Self>) {}
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            self.flags
                .lock_was_free
                .store(self.shared.try_lock().is_ok(), Ordering::SeqCst);
            self.flags.dropped.store(true, Ordering::SeqCst);
        }
    }

    fn handle() -> Handle {
        let conn = Connection::new(ServerSettings::default(), PeerType::Server);
        Arc::new(Mutex::new(Shared::new(conn, true, false)))
    }

    fn probe(shared: &Handle) -> (Waker, Arc<Flags>) {
        let flags = Arc::new(Flags::default());
        let waker = Waker::from(Arc::new(Probe {
            shared: shared.clone(),
            flags: flags.clone(),
        }));
        (waker, flags)
    }

    fn assert_dropped_unlocked(flags: &Flags, what: &str) {
        assert!(flags.dropped.load(Ordering::SeqCst), "{what}: not dropped");
        assert!(
            flags.lock_was_free.load(Ordering::SeqCst),
            "{what}: dropped while the shared mutex was held"
        );
    }

    /// Polls `fut` once with `waker`.
    fn poll_once<F: Future + Unpin>(fut: &mut F, waker: &Waker) -> Poll<F::Output> {
        Pin::new(fut).poll(&mut Context::from_waker(waker))
    }

    #[test]
    fn a_replaced_reader_waker_is_dropped_outside_the_lock() {
        let shared = handle();
        lock(&shared).add_stream(1, false);
        let mut body = RecvStream {
            shared: shared.clone(),
            id: 1,
        };
        let mut next = Box::pin(body.next());
        let (waker, flags) = probe(&shared);
        assert!(poll_once(&mut next, &waker).is_pending());
        drop(waker); // the stream now holds the only reference
        assert!(poll_once(&mut next, Waker::noop()).is_pending());
        assert_dropped_unlocked(&flags, "reader waker");
    }

    #[test]
    fn a_replaced_response_waker_is_dropped_outside_the_lock() {
        let shared = handle();
        lock(&shared).add_stream(1, true);
        let mut resp = ResponseFuture {
            shared: shared.clone(),
            id: 1,
            taken: false,
        };
        let (waker, flags) = probe(&shared);
        assert!(poll_once(&mut resp, &waker).is_pending());
        drop(waker);
        assert!(poll_once(&mut resp, Waker::noop()).is_pending());
        assert_dropped_unlocked(&flags, "response waker");
    }

    #[test]
    fn a_replaced_accept_waker_is_dropped_outside_the_lock() {
        let shared = handle();
        let mut acceptor = Acceptor {
            shared: shared.clone(),
        };
        let mut accept = Box::pin(acceptor.accept());
        let (waker, flags) = probe(&shared);
        assert!(poll_once(&mut accept, &waker).is_pending());
        drop(waker);
        assert!(poll_once(&mut accept, Waker::noop()).is_pending());
        assert_dropped_unlocked(&flags, "accept waker");
    }

    fn client_handle(shared: &Handle) -> ClientHandle {
        ClientHandle {
            shared: shared.clone(),
            _token: Arc::new(Token(shared.clone())),
        }
    }

    #[test]
    fn a_replaced_ping_waker_is_dropped_outside_the_lock() {
        let shared = handle();
        let client = client_handle(&shared);
        let mut ping = Box::pin(client.ping());
        let (waker, flags) = probe(&shared);
        assert!(poll_once(&mut ping, &waker).is_pending());
        drop(waker);
        assert!(poll_once(&mut ping, Waker::noop()).is_pending());
        assert_dropped_unlocked(&flags, "ping waker");
    }

    #[test]
    fn a_ping_cancelled_before_its_ack_is_forgotten() {
        let shared = handle();
        let client = client_handle(&shared);
        {
            let mut ping = Box::pin(client.ping());
            assert!(poll_once(&mut ping, Waker::noop()).is_pending());
            assert_eq!(lock(&shared).pings.len(), 1);
        } // cancelled
        assert!(lock(&shared).pings.is_empty());
    }

    #[test]
    fn a_ping_acked_but_never_repolled_is_forgotten() {
        let shared = handle();
        let client = client_handle(&shared);
        {
            let mut ping = Box::pin(client.ping());
            assert!(poll_once(&mut ping, Waker::noop()).is_pending());
            let key = *lock(&shared).pings.keys().next().unwrap();
            lock(&shared)
                .on_frame(Frame::Ping(PingFrame {
                    ack: true,
                    opaque_data: key.to_be_bytes(),
                }))
                .unwrap();
            assert!(lock(&shared).pings[&key].acked);
        } // dropped after the ACK, before it was polled again
        assert!(lock(&shared).pings.is_empty());
    }

    #[test]
    fn a_ping_cancelled_with_a_stored_waker_drops_it_outside_the_lock() {
        let shared = handle();
        let client = client_handle(&shared);
        let (waker, flags) = probe(&shared);
        {
            let mut ping = Box::pin(client.ping());
            assert!(poll_once(&mut ping, &waker).is_pending());
            drop(waker);
        }
        assert_dropped_unlocked(&flags, "cancelled ping's waker");
    }
}
