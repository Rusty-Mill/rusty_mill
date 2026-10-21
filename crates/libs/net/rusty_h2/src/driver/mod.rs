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
                let old = s.recv_waker.replace(cx.waker().clone());
                sh.trash.extend(old);
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
        let old = s.headers_waker.replace(cx.waker().clone());
        sh.trash.extend(old);
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
        // However this future ends (answered, closed, or dropped early), the
        // ping stops being tracked.
        let _tracked = PingGuard {
            shared: self.shared.clone(),
            key,
        };
        poll_fn(|cx| {
            let mut sh = lock(&self.shared);
            if sh.pings.get(&key).is_some_and(|p| p.acked) {
                return Poll::Ready(Ok(()));
            }
            if sh.closed {
                return Poll::Ready(Err(Error::Closed));
            }
            let old = sh
                .pings
                .get_mut(&key)
                .and_then(|p| p.waker.replace(cx.waker().clone()));
            sh.trash.extend(old);
            Poll::Pending
        })
        .await
    }
}

/// Forgets a ping when its future ends or is dropped.
struct PingGuard {
    shared: Handle,
    key: u64,
}

impl Drop for PingGuard {
    fn drop(&mut self) {
        lock(&self.shared).forget_ping(self.key);
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
            let old = sh.accept_waker.replace(cx.waker().clone());
            sh.trash.extend(old);
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
        wbuf: Vec::new(),
        wpos: 0,
        accepted: 0,
        flushed: 0,
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

    // Not `Waker::noop()`: the point is what happens when it is destroyed.
    #[allow(clippy::manual_noop_waker)]
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

    /// A transport with a stored waker per direction, as real ones have, and
    /// a budget of bytes a write will take.
    #[derive(Default)]
    struct StubState {
        /// Bytes `poll_read` hands out; once used up it is pending.
        input: Vec<u8>,
        read_pos: usize,
        read_waker: Option<Waker>,
        write_waker: Option<Waker>,
        budget: usize,
        taken: usize,
        /// Once `input` is used up, reads fail with this instead of waiting.
        read_error: Option<std::io::ErrorKind>,
        /// Writes fail with this instead of using the budget.
        write_error: Option<std::io::ErrorKind>,
        /// Flushes fail with this.
        flush_error: Option<std::io::ErrorKind>,
        /// Shutdown fails with this.
        shutdown_error: Option<std::io::ErrorKind>,
        /// Writes are accepted but a flush stays pending while this is set:
        /// output the transport holds without having delivered it.
        hold_flush: bool,
    }

    #[derive(Clone, Default)]
    struct Stub(Arc<Mutex<StubState>>);

    impl rusty_tokio::io::AsyncRead for Stub {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut rusty_tokio::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let mut st = self.0.lock().unwrap();
            if st.read_pos < st.input.len() {
                let n = buf.remaining().min(st.input.len() - st.read_pos);
                let start = st.read_pos;
                buf.unfilled_mut()[..n].copy_from_slice(&st.input[start..start + n]);
                buf.advance(n);
                st.read_pos += n;
                return Poll::Ready(Ok(()));
            }
            if let Some(kind) = st.read_error {
                return Poll::Ready(Err(kind.into()));
            }
            // Replacing the stored waker destroys the previous one, right here.
            st.read_waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }

    impl rusty_tokio::io::AsyncWrite for Stub {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let mut st = self.0.lock().unwrap();
            if let Some(kind) = st.write_error {
                return Poll::Ready(Err(kind.into()));
            }
            st.write_waker = Some(cx.waker().clone());
            let n = buf.len().min(st.budget);
            if n == 0 {
                return Poll::Pending;
            }
            st.budget -= n;
            st.taken += n;
            Poll::Ready(Ok(n))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            let st = self.0.lock().unwrap();
            if let Some(kind) = st.flush_error {
                return Poll::Ready(Err(kind.into()));
            }
            if st.hold_flush {
                return Poll::Pending;
            }
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            match self.0.lock().unwrap().shutdown_error {
                Some(kind) => Poll::Ready(Err(kind.into())),
                None => Poll::Ready(Ok(())),
            }
        }
    }

    #[test]
    fn the_transport_is_polled_outside_the_lock() {
        let stub = Stub::default();
        let (_client, mut driver) = client(stub, crate::client::builder::Builder::new());
        let shared = driver.shared.clone();
        let (waker, flags) = probe(&shared);
        // The driver is polled by a task whose waker the transport stores.
        assert!(poll_once(&mut driver, &waker).is_pending());
        drop(waker);
        // Polled again by another waker, the transport replaces (and destroys)
        // the first while the driver is inside its write.
        assert!(poll_once(&mut driver, Waker::noop()).is_pending());
        assert_dropped_unlocked(&flags, "the transport's stored waker");
    }

    #[test]
    fn output_the_transport_has_not_taken_still_counts_as_backlog() {
        let stub = Stub::default();
        let (_client, mut driver) = client(stub.clone(), crate::client::builder::Builder::new());
        let shared = driver.shared.clone();
        assert!(poll_once(&mut driver, Waker::noop()).is_pending());
        let queued = lock(&shared).backlog();
        assert!(queued > 0, "the preface and SETTINGS are waiting");
        // The transport takes a little; the rest is still backlog, so a writer
        // that checks it keeps waiting (backpressure).
        stub.0.lock().unwrap().budget = 10;
        assert!(poll_once(&mut driver, Waker::noop()).is_pending());
        assert_eq!(stub.0.lock().unwrap().taken, 10);
        assert_eq!(lock(&shared).backlog(), queued - 10);
        // More output queued meanwhile is counted on top, in order.
        lock(&shared).queue([Frame::Ping(PingFrame {
            ack: false,
            opaque_data: [1; 8],
        })]);
        assert_eq!(lock(&shared).backlog(), queued - 10 + 17);
    }

    #[test]
    fn a_ping_flood_is_not_read_faster_than_its_acks_can_be_written() {
        use crate::frame::SettingsFrame;
        const PINGS: usize = 100_000;
        let mut input = crate::CONNECTION_PREFACE.to_vec();
        Frame::Settings(SettingsFrame {
            ack: false,
            settings: vec![],
        })
        .encode(&mut input);
        for _ in 0..PINGS {
            Frame::Ping(PingFrame {
                ack: false,
                opaque_data: [3; 8],
            })
            .encode(&mut input);
        }
        let total = input.len();
        let stub = Stub::default();
        stub.0.lock().unwrap().input = input;
        // The transport takes no output at all: its acks pile up.
        let (_acceptor, mut driver) = server(stub.clone(), crate::server::builder::Builder::new());
        let shared = driver.shared.clone();
        for _ in 0..8 {
            assert!(poll_once(&mut driver, Waker::noop()).is_pending());
        }
        let read = stub.0.lock().unwrap().read_pos;
        assert!(
            read < total,
            "the driver must stop reading, not read it all"
        );
        let backlog = lock(&shared).backlog();
        assert!(
            (shared::OUT_HIGH..shared::OUT_HIGH + 128 * 1024).contains(&backlog),
            "unwritten output is capped near the limit, not {backlog}"
        );
        // Once the transport takes output again, the rest is read and answered.
        // (Polled in a loop with a no-op waker: wake-ups are not exercised here.)
        stub.0.lock().unwrap().budget = usize::MAX / 2;
        for _ in 0..1_000 {
            let _ = poll_once(&mut driver, Waker::noop());
            if stub.0.lock().unwrap().read_pos == total {
                break;
            }
        }
        let st = stub.0.lock().unwrap();
        assert_eq!(st.read_pos, total);
        assert!(st.taken >= PINGS * 17, "bytes for every ping were written");
    }

    // ---- a peer that resets the connection after saying goodbye -----------
    //
    // Windows reports a peer that closed with unread data as a reset
    // (`ConnectionReset`) or an abort (`ConnectionAborted`), on the read or on
    // the write that follows. Both are a clean wind-down only when the peer had
    // said GOAWAY and nothing it needed was left undelivered; everything else
    // is still a failure. These tests drive the driver by hand over `Stub`, so
    // they are deterministic and the same on every OS.

    use std::io::ErrorKind;

    /// The errors a transport gives for a peer that is gone.
    const PEER_GONE: [ErrorKind; 4] = [
        ErrorKind::BrokenPipe,
        ErrorKind::ConnectionReset,
        ErrorKind::NotConnected,
        ErrorKind::ConnectionAborted,
    ];

    fn goaway() -> Frame {
        Frame::GoAway(crate::frame::GoAwayFrame {
            last_stream_id: 0,
            error_code: ErrorCode::NoError,
            debug_data: vec![],
        })
    }

    fn request_headers(id: u32, end_stream: bool) -> Frame {
        let mut block = Vec::new();
        crate::hpack::Encoder::default().encode(
            &[
                HeaderField::new(":method", "POST"),
                HeaderField::new(":scheme", "http"),
                HeaderField::new(":path", "/x"),
                HeaderField::new(":authority", "localhost"),
            ],
            &mut block,
        );
        Frame::Headers(crate::frame::HeadersFrame {
            stream_id: id,
            end_stream,
            end_headers: true,
            priority: None,
            header_block_fragment: block,
        })
    }

    /// A client's opening bytes (preface and empty SETTINGS) then `frames`.
    fn peer_input(frames: &[Frame]) -> Vec<u8> {
        let mut input = crate::CONNECTION_PREFACE.to_vec();
        Frame::Settings(crate::frame::SettingsFrame {
            ack: false,
            settings: vec![],
        })
        .encode(&mut input);
        for f in frames {
            f.encode(&mut input);
        }
        input
    }

    /// A server driver over a [`Stub`] that has `input` to read, and writes
    /// without limit.
    fn stub_server(input: Vec<u8>) -> (Stub, Acceptor, ConnectionDriver<Stub>) {
        let stub = Stub::default();
        {
            let mut st = stub.0.lock().unwrap();
            st.input = input;
            st.budget = usize::MAX / 2;
        }
        let (acceptor, driver) = server(stub.clone(), crate::server::builder::Builder::new());
        (stub, acceptor, driver)
    }

    /// Polls `driver` until it ends; a driver that is still waiting after a
    /// few polls is stuck.
    fn finish<T: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
        driver: &mut ConnectionDriver<T>,
    ) -> Result<(), Error> {
        for _ in 0..8 {
            if let Poll::Ready(r) = poll_once(driver, Waker::noop()) {
                return r;
            }
        }
        panic!("the driver did not finish");
    }

    /// The transport operations a peer that has gone can fail.
    #[derive(Clone, Copy, Debug)]
    enum Op {
        Write,
        Flush,
        Shutdown,
    }

    const OPS: [Op; 3] = [Op::Write, Op::Flush, Op::Shutdown];

    fn fail(stub: &Stub, op: Op, kind: ErrorKind) {
        let mut st = stub.0.lock().unwrap();
        match op {
            Op::Write => st.write_error = Some(kind),
            Op::Flush => st.flush_error = Some(kind),
            Op::Shutdown => st.shutdown_error = Some(kind),
        }
    }

    #[test]
    fn a_write_flush_or_shutdown_a_departed_peer_refuses_after_its_goodbye_is_not_a_failure() {
        for op in OPS {
            for kind in PEER_GONE {
                let (stub, _acceptor, mut driver) = stub_server(peer_input(&[goaway()]));
                fail(&stub, op, kind);
                let r = finish(&mut driver);
                assert!(r.is_ok(), "{op:?} {kind:?}: {r:?}");
            }
        }
    }

    #[test]
    fn a_failure_to_write_flush_or_shut_down_without_any_goodbye_is_still_a_failure() {
        // This side winds down on its own (every handle dropped); the peer has
        // said nothing and the input has not ended, so nothing says it left.
        for op in OPS {
            for kind in PEER_GONE {
                let stub = Stub::default();
                stub.0.lock().unwrap().budget = usize::MAX / 2;
                let (handle, mut driver) =
                    client(stub.clone(), crate::client::builder::Builder::new());
                drop(handle);
                fail(&stub, op, kind);
                let r = finish(&mut driver);
                assert!(
                    matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                    "{op:?} {kind:?}: {r:?}"
                );
            }
        }
    }

    /// Input that is not whole, after a goodbye: a frame cut short, or a
    /// header block still waiting for its CONTINUATION.
    fn incomplete_inputs() -> Vec<(&'static str, Vec<u8>)> {
        let mut cut = peer_input(&[goaway()]);
        cut.extend_from_slice(&[0, 0, 4]); // the start of a frame header
        let mut unfinished = peer_input(&[goaway()]);
        let Frame::Headers(mut headers) = request_headers(1, false) else {
            unreachable!()
        };
        headers.end_headers = false;
        Frame::Headers(headers).encode(&mut unfinished);
        vec![
            ("a frame cut short", cut),
            ("an unfinished header block", unfinished),
        ]
    }

    #[test]
    fn incomplete_input_after_a_goodbye_is_not_clean_on_the_write_flush_and_shutdown_paths() {
        // The read stays pending (the peer has not reset), the driver has
        // wound down on the GOAWAY, and then the final operation fails.
        for (what, input) in incomplete_inputs() {
            for op in OPS {
                for kind in PEER_GONE {
                    let (stub, _acceptor, mut driver) = stub_server(input.clone());
                    fail(&stub, op, kind);
                    let r = finish(&mut driver);
                    assert!(
                        matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                        "{what}, {op:?} {kind:?}: {r:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_reset_read_with_incomplete_input_is_a_failure_even_after_a_goodbye() {
        for (what, input) in incomplete_inputs() {
            for kind in PEER_GONE {
                let (stub, _acceptor, mut driver) = stub_server(input.clone());
                stub.0.lock().unwrap().read_error = Some(kind);
                let r = finish(&mut driver);
                assert!(
                    matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                    "{what}, {kind:?}: {r:?}"
                );
            }
        }
    }

    #[test]
    fn a_reset_read_after_the_peers_goodbye_is_a_clean_wind_down() {
        for kind in PEER_GONE {
            let (stub, _acceptor, mut driver) = stub_server(peer_input(&[goaway()]));
            stub.0.lock().unwrap().read_error = Some(kind);
            let r = finish(&mut driver);
            assert!(r.is_ok(), "read {kind:?}: {r:?}");
        }
    }

    #[test]
    fn a_reset_read_without_a_goodbye_is_a_failure() {
        for kind in PEER_GONE {
            let (stub, _acceptor, mut driver) = stub_server(peer_input(&[]));
            stub.0.lock().unwrap().read_error = Some(kind);
            let r = finish(&mut driver);
            assert!(
                matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                "{kind:?}: {r:?}"
            );
        }
    }

    #[test]
    fn an_unrelated_read_error_after_a_goodbye_is_a_failure() {
        for kind in [
            ErrorKind::TimedOut,
            ErrorKind::PermissionDenied,
            ErrorKind::Other,
        ] {
            let (stub, _acceptor, mut driver) = stub_server(peer_input(&[goaway()]));
            stub.0.lock().unwrap().read_error = Some(kind);
            let r = finish(&mut driver);
            assert!(
                matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                "{kind:?}: {r:?}"
            );
        }
    }

    #[test]
    fn a_reset_read_in_the_middle_of_a_frame_is_a_failure_even_after_a_goodbye() {
        for kind in PEER_GONE {
            let mut input = peer_input(&[goaway()]);
            input.extend_from_slice(&[0, 0, 4]); // the start of a frame header
            let (stub, _acceptor, mut driver) = stub_server(input);
            stub.0.lock().unwrap().read_error = Some(kind);
            let r = finish(&mut driver);
            assert!(
                matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                "{kind:?}: {r:?}"
            );
        }
    }

    #[test]
    fn a_reset_read_with_a_request_unaccepted_is_a_failure_even_after_a_goodbye() {
        for kind in PEER_GONE {
            let (stub, _acceptor, mut driver) =
                stub_server(peer_input(&[request_headers(1, false), goaway()]));
            stub.0.lock().unwrap().read_error = Some(kind);
            let r = finish(&mut driver);
            assert!(
                matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                "{kind:?}: {r:?}"
            );
        }
    }

    #[test]
    fn a_reset_read_with_an_accepted_call_unanswered_is_a_failure_even_after_a_goodbye() {
        for kind in PEER_GONE {
            let (stub, mut acceptor, mut driver) =
                stub_server(peer_input(&[request_headers(1, false), goaway()]));
            assert!(poll_once(&mut driver, Waker::noop()).is_pending());
            let mut accept = Box::pin(acceptor.accept());
            let Poll::Ready(Some(Ok(_call))) = poll_once(&mut accept, Waker::noop()) else {
                panic!("the request is accepted");
            };
            // The call is held open, unanswered, while the peer resets.
            {
                let mut st = stub.0.lock().unwrap();
                st.read_error = Some(kind);
                st.write_error = Some(kind);
            }
            let r = finish(&mut driver);
            assert!(
                matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                "{kind:?}: {r:?}"
            );
        }
    }

    #[test]
    fn a_write_or_flush_that_fails_with_a_response_undelivered_is_a_failure_even_after_a_goodbye() {
        // The driver has wound down on the peer's GOAWAY, but the response it
        // owes is still in its own buffer or the transport's. Losing that is
        // not losing housekeeping.
        for op in [Op::Write, Op::Flush] {
            for kind in PEER_GONE {
                let (stub, mut acceptor, mut driver) =
                    stub_server(peer_input(&[request_headers(1, true), goaway()]));
                stub.0.lock().unwrap().hold_flush = true;
                assert!(poll_once(&mut driver, Waker::noop()).is_pending());
                let mut accept = Box::pin(acceptor.accept());
                let Poll::Ready(Some(Ok((_request, respond)))) =
                    poll_once(&mut accept, Waker::noop())
                else {
                    panic!("the request is accepted");
                };
                let mut reply =
                    Box::pin(respond.send_response(vec![HeaderField::new(":status", "200")], true));
                assert!(matches!(
                    poll_once(&mut reply, Waker::noop()),
                    Poll::Ready(Ok(_))
                ));
                match op {
                    // The transport takes the response, holds it, then fails.
                    Op::Flush => {
                        assert!(poll_once(&mut driver, Waker::noop()).is_pending());
                        let mut st = stub.0.lock().unwrap();
                        st.hold_flush = false;
                        st.flush_error = Some(kind);
                    }
                    // The transport refuses the response outright.
                    _ => fail(&stub, Op::Write, kind),
                }
                let r = finish(&mut driver);
                assert!(
                    matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                    "{op:?} {kind:?}: {r:?}"
                );
            }
        }
    }

    #[test]
    fn a_reset_read_with_a_response_accepted_but_not_flushed_is_a_failure_even_after_a_goodbye() {
        // Only the read fails. Writes are accepted, so nothing here says they
        // will fail; but the response is held by the transport, not delivered.
        for kind in PEER_GONE {
            let (stub, mut acceptor, mut driver) =
                stub_server(peer_input(&[request_headers(1, true), goaway()]));
            stub.0.lock().unwrap().hold_flush = true;
            assert!(poll_once(&mut driver, Waker::noop()).is_pending());
            let mut accept = Box::pin(acceptor.accept());
            let Poll::Ready(Some(Ok((_request, respond)))) = poll_once(&mut accept, Waker::noop())
            else {
                panic!("the request is accepted");
            };
            let mut reply =
                Box::pin(respond.send_response(vec![HeaderField::new(":status", "200")], true));
            assert!(matches!(
                poll_once(&mut reply, Waker::noop()),
                Poll::Ready(Ok(_))
            ));
            // The read fails in the very poll that finds the response queued,
            // before any of it is flushed (a poll that found nothing to read
            // would let the driver wind down cleanly on the GOAWAY first).
            stub.0.lock().unwrap().read_error = Some(kind);
            assert!(poll_once(&mut driver, Waker::noop()).is_pending());
            // The transport has the response and holds it. Once it lets go,
            // the connection ends with the read error, not as a clean wind-down.
            stub.0.lock().unwrap().hold_flush = false;
            let r = finish(&mut driver);
            assert!(
                matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                "{kind:?}: {r:?}"
            );
        }
    }

    #[test]
    fn a_reset_read_with_a_response_unsent_and_writes_failing_too_is_a_failure_even_after_a_goodbye(
    ) {
        for kind in PEER_GONE {
            let (stub, mut acceptor, mut driver) =
                stub_server(peer_input(&[request_headers(1, true), goaway()]));
            // Nothing can be written yet.
            stub.0.lock().unwrap().budget = 0;
            assert!(poll_once(&mut driver, Waker::noop()).is_pending());
            let mut accept = Box::pin(acceptor.accept());
            let Poll::Ready(Some(Ok((_request, respond)))) = poll_once(&mut accept, Waker::noop())
            else {
                panic!("the request is accepted");
            };
            let mut reply =
                Box::pin(respond.send_response(vec![HeaderField::new(":status", "200")], true));
            assert!(matches!(
                poll_once(&mut reply, Waker::noop()),
                Poll::Ready(Ok(_))
            ));
            // The response is queued, undelivered; then the peer resets, which
            // fails the writes as well as the reads.
            {
                let mut st = stub.0.lock().unwrap();
                st.read_error = Some(kind);
                st.write_error = Some(kind);
            }
            let r = finish(&mut driver);
            assert!(
                matches!(r, Err(Error::Io(ref e)) if e.kind() == kind),
                "{kind:?}: {r:?}"
            );
        }
    }

    #[test]
    fn a_reset_read_after_a_delivered_response_and_a_goodbye_is_clean() {
        for kind in PEER_GONE {
            let (stub, mut acceptor, mut driver) =
                stub_server(peer_input(&[request_headers(1, true)]));
            assert!(poll_once(&mut driver, Waker::noop()).is_pending());
            let mut accept = Box::pin(acceptor.accept());
            let Poll::Ready(Some(Ok((_request, respond)))) = poll_once(&mut accept, Waker::noop())
            else {
                panic!("the request is accepted");
            };
            let mut reply =
                Box::pin(respond.send_response(vec![HeaderField::new(":status", "200")], true));
            assert!(matches!(
                poll_once(&mut reply, Waker::noop()),
                Poll::Ready(Ok(_))
            ));
            // The response goes out; then the peer says goodbye and resets.
            assert!(poll_once(&mut driver, Waker::noop()).is_pending());
            {
                let mut st = stub.0.lock().unwrap();
                goaway().encode(&mut st.input);
                st.read_error = Some(kind);
            }
            let r = finish(&mut driver);
            assert!(r.is_ok(), "{kind:?}: {r:?}");
        }
    }
}
