//! The server: HTTP/1.1 over `rusty_tokio`, plain or TLS, answering with a
//! [`Router`]. The only module that touches a socket or a runtime.
//!
//! Each connection is one task serving requests in order (keep-alive, no
//! pipelining in parallel). A request's head and body are bounded and read
//! whole before routing. Shutdown is graceful: no new connections, idle
//! connections close, a request in flight finishes, and a streamed body
//! (SSE) is ended cleanly with its terminating chunk.

use std::future::{poll_fn, Future};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusty_http::async_tokio::AsyncTransport;
use rusty_http::body::{request_framing, Framing};
use rusty_http::head::{RequestHead, ResponseHead};
use rusty_http::{HeaderMap, Method, StatusCode, TransportError, Version};
use rusty_tls::{AsyncTlsServerStream, TlsAcceptor};
use rusty_tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, TcpListener, TcpStream};
use rusty_tokio::sync::{OwnedSemaphorePermit, Semaphore};
use rusty_tokio::task::JoinHandle;
use rusty_tokio::time::{sleep, timeout, timeout_at};

use crate::body::{BodyStream, Inner};
use crate::request::Request;
use crate::response::Response;
use crate::routing::Router;
use crate::stop::Stop;

/// How [`serve`] runs.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// The most connections served at once; the next is closed unserved.
    /// An SSE stream holds its connection for its whole life.
    pub max_connections: usize,
    /// The largest request head (request line and headers); larger is 431.
    pub max_head_bytes: usize,
    /// The largest request body, read whole before routing; larger is 413.
    pub max_body_bytes: u64,
    /// How long a connection may wait for its next request, and the limit
    /// on each read of a request and each write of a response.
    pub idle_timeout: Duration,
    /// The limit on a TLS handshake.
    pub handshake_timeout: Duration,
}

impl Default for ServeOptions {
    /// 1024 connections, an 8 KiB head, a 2 MiB body (axum's default body
    /// limit), 60 s idle and a 10 s handshake.
    fn default() -> Self {
        ServeOptions {
            max_connections: 1024,
            max_head_bytes: rusty_http::head::DEFAULT_MAX_HEAD_LEN,
            max_body_bytes: 2 << 20,
            idle_timeout: Duration::from_secs(60),
            handshake_timeout: Duration::from_secs(10),
        }
    }
}

/// A running server. Dropping it begins the same graceful shutdown as
/// [`Server::shutdown`] without waiting for it.
pub struct Server {
    local_addr: SocketAddr,
    stop: Arc<Stop>,
    task: Option<JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.stop();
    }
}

impl Server {
    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stops accepting, closes idle connections, ends streamed bodies, and
    /// waits until requests in flight have been answered.
    pub async fn shutdown(mut self) {
        self.stop.stop();
        if let Some(task) = self.task.take() {
            // A failed join means the accept loop panicked; it is gone
            // either way, which is what shutting down asks for.
            let _ = task.await;
        }
    }

    /// Serves until `signal` completes, then shuts down as
    /// [`Server::shutdown`]: axum's `with_graceful_shutdown`.
    pub async fn shutdown_on(self, signal: impl Future<Output = ()>) {
        signal.await;
        self.shutdown().await;
    }
}

/// Serves `router` on `listener`: plain HTTP/1.1 when `tls` is `None`,
/// otherwise HTTP/1.1 over TLS. Must be called inside a `rusty_tokio`
/// runtime; the server runs on its tasks.
///
/// # Errors
///
/// When the listener's address cannot be read.
pub fn serve(
    listener: TcpListener,
    tls: Option<TlsAcceptor>,
    options: ServeOptions,
    router: Router,
) -> io::Result<Server> {
    let local_addr = listener.local_addr()?;
    let stop = Arc::new(Stop::default());
    let max = options.max_connections.clamp(1, u32::MAX as usize);
    let shared = Arc::new(Shared {
        router,
        tls,
        options,
    });
    Ok(Server {
        local_addr,
        task: Some(rusty_tokio::spawn(accept_loop(
            listener,
            shared,
            stop.clone(),
            max,
        ))),
        stop,
    })
}

struct Shared {
    router: Router,
    tls: Option<TlsAcceptor>,
    options: ServeOptions,
}

/// Waits for `work`, or for the stop signal: `None` once stopped. The stop
/// signal is polled first so that always-ready work cannot starve it.
async fn until_stopped<F: Future>(work: F, stop: &Arc<Stop>) -> Option<F::Output> {
    if stop.is_stopped() {
        return None;
    }
    rusty_tokio::select! {
        _ = stop.wait() => None,
        w = work => Some(w),
    }
}

/// Accepts connections until told to stop, then waits for them all to end.
async fn accept_loop(listener: TcpListener, shared: Arc<Shared>, stop: Arc<Stop>, max: usize) {
    let permits = Arc::new(Semaphore::new(max));
    loop {
        let Some(accepted) = until_stopped(listener.accept(), &stop).await else {
            break;
        };
        let Ok((tcp, _)) = accepted else {
            // Out of descriptors, say: back off rather than spin.
            sleep(Duration::from_millis(10)).await;
            continue;
        };
        // At the cap the connection is closed unserved.
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        rusty_tokio::spawn(connection(tcp, shared.clone(), stop.clone(), permit));
    }
    drop(listener);
    // Every connection holds a permit until it ends.
    let _ = permits.acquire_many(max as u32).await;
}

/// One accepted connection: the TLS handshake if any, then HTTP/1.1.
async fn connection(
    tcp: TcpStream,
    shared: Arc<Shared>,
    stop: Arc<Stop>,
    _permit: OwnedSemaphorePermit,
) {
    let _ = tcp.set_nodelay(true);
    let Some(acceptor) = shared.tls.clone() else {
        return run(tcp, &shared, &stop).await;
    };
    let deadline = Instant::now() + shared.options.handshake_timeout;
    if let Ok(Ok(io)) = timeout_at(deadline, handshake(acceptor, tcp)).await {
        run(io, &shared, &stop).await;
    }
}

async fn handshake(
    acceptor: TlsAcceptor,
    tcp: TcpStream,
) -> Result<AsyncTlsServerStream<TcpStream>, rusty_tls::Error> {
    let mut tls = acceptor.accept_async(tcp)?;
    tls.complete_handshake().await?;
    Ok(tls)
}

/// Serves requests on `io` in order until the client is done, a limit is
/// hit, or the server stops.
async fn run<T>(io: T, shared: &Shared, stop: &Arc<Stop>)
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    let options = &shared.options;
    let mut conn = Conn {
        io: AsyncTransport::new(io),
        idle: options.idle_timeout,
    };
    loop {
        let next = conn.io.read_request_head(options.max_head_bytes);
        let head = match until_stopped(timeout(options.idle_timeout, next), stop).await {
            Some(Ok(Ok(head))) => head,
            Some(Ok(Err(e))) => return conn.refuse(&e).await,
            // Stopped between requests, or idle too long.
            None | Some(Err(_)) => return,
        };
        let reply = Reply {
            head_only: head.method == Method::Head,
            keep_alive: wants_keep_alive(&head),
            chunked: head.version == Version::Http11,
        };
        let req = match conn.read_request(head, options).await {
            Ok(req) => req,
            Err(e) => return conn.refuse(&e).await,
        };
        let response = shared.router.call(req).await;
        let reusable = conn.respond(response, reply, stop).await;
        if !reusable {
            return;
        }
    }
}

fn wants_keep_alive(head: &RequestHead) -> bool {
    let close = head
        .headers
        .get_all("connection")
        .flat_map(|v| v.split(','))
        .any(|token| token.trim().eq_ignore_ascii_case("close"));
    head.version == Version::Http11 && !close
}

fn expects_continue(head: &RequestHead) -> bool {
    head.version == Version::Http11
        && head
            .headers
            .get("expect")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("100-continue"))
}

/// How to answer the request just read.
#[derive(Debug, Clone, Copy)]
struct Reply {
    head_only: bool,
    keep_alive: bool,
    /// HTTP/1.1: a stream may be sent chunked.
    chunked: bool,
}

fn chunk_frame(chunk: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(chunk.len() + 16);
    rusty_http::body::write_chunk(&mut out, chunk);
    out
}

struct Conn<T> {
    io: AsyncTransport<T>,
    idle: Duration,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Conn<T> {
    /// Reads the body after `head`, honouring `Expect: 100-continue`.
    async fn read_request(
        &mut self,
        head: RequestHead,
        options: &ServeOptions,
    ) -> Result<Request, TransportError> {
        let framing = request_framing(&head.headers)?;
        if let Framing::ContentLength(len) = framing {
            if len > options.max_body_bytes {
                return Err(rusty_http::Error::BodyTooLarge.into());
            }
        }
        if framing != Framing::None && expects_continue(&head) {
            self.write(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
        }
        let read = self
            .io
            .read_body_with_limit(framing, options.max_body_bytes);
        let body = timeout(self.idle, read).await.map_err(timed_out)??;
        Ok(Request::new(head.method, head.target)
            .with_headers(head.headers)
            .with_body(body))
    }

    /// Answers a request that could not be read, then the connection ends.
    /// A connection that simply went away gets nothing.
    async fn refuse(&mut self, error: &TransportError) {
        let status = match error {
            TransportError::Io(_) => return,
            TransportError::Http(rusty_http::Error::HeadTooLarge) => StatusCode::from_u16(431),
            TransportError::Http(rusty_http::Error::BodyTooLarge) => StatusCode::PAYLOAD_TOO_LARGE,
            TransportError::Http(_) => StatusCode::BAD_REQUEST,
        };
        let mut headers = HeaderMap::new();
        set(&mut headers, "connection", "close");
        set(&mut headers, "content-length", "0");
        // The connection ends either way; a failed write changes nothing.
        let _ = self.write(&head_bytes(status, &headers)).await;
    }

    /// Writes `response`; whether the connection can serve another request.
    async fn respond(&mut self, response: Response, reply: Reply, stop: &Arc<Stop>) -> bool {
        let Reply {
            head_only,
            keep_alive,
            chunked,
        } = reply;
        let Response {
            status,
            mut headers,
            body,
        } = response;
        // The router answers HEAD bodiless, with the GET body's length as
        // `Content-Length` when it is known: that header is the answer.
        let head_length = head_only
            .then(|| headers.get("content-length").map(str::to_string))
            .flatten();
        for name in ["content-length", "transfer-encoding", "connection"] {
            headers.remove(name);
        }
        if !keep_alive {
            set(&mut headers, "connection", "close");
        }
        let code = status.as_u16();
        let bodiless = status.is_informational() || code == 204 || code == 304;
        match body.0 {
            Inner::Full(bytes) => {
                // 1xx and 204 may not carry a length (RFC 9110 §8.6). A HEAD
                // answer carries the GET body's length only when the router
                // knew it, never the length of the body it stripped.
                let length = if status.is_informational() || code == 204 {
                    None
                } else if head_only {
                    head_length
                } else if !bodiless {
                    Some(bytes.len().to_string())
                } else {
                    None
                };
                if let Some(length) = &length {
                    set(&mut headers, "content-length", length);
                }
                let send_body = !(head_only || bodiless);
                let mut out = head_bytes(status, &headers);
                if send_body {
                    out.extend_from_slice(&bytes);
                }
                self.write(&out).await.is_ok() && keep_alive
            }
            Inner::Stream(_) if head_only || bodiless => {
                let out = head_bytes(status, &headers);
                self.write(&out).await.is_ok() && keep_alive
            }
            Inner::Stream(stream) if chunked => {
                set(&mut headers, "transfer-encoding", "chunked");
                if self.write(&head_bytes(status, &headers)).await.is_err() {
                    return false;
                }
                let ended = self.stream(stream, stop, chunk_frame).await;
                // Ended by the stream or by shutdown: close the body
                // properly either way. A failed stream gets no terminator,
                // so the client sees the body truncated.
                ended && self.write(b"0\r\n\r\n").await.is_ok() && keep_alive && !stop.is_stopped()
            }
            // HTTP/1.0 has no chunked coding (RFC 9112 §6.1): the bytes go
            // as they are and closing the connection ends the body.
            Inner::Stream(stream) => {
                if self.write(&head_bytes(status, &headers)).await.is_ok() {
                    self.stream(stream, stop, <[u8]>::to_vec).await;
                }
                false
            }
        }
    }

    /// Writes each chunk of `stream`, framed by `frame`, until it ends or
    /// the server stops (`true`), or it fails or a write fails (`false`).
    async fn stream(
        &mut self,
        mut stream: BodyStream,
        stop: &Arc<Stop>,
        frame: fn(&[u8]) -> Vec<u8>,
    ) -> bool {
        loop {
            let next = poll_fn(|cx| stream.as_mut().poll_next(cx));
            let chunk = match until_stopped(next, stop).await {
                None | Some(None) => return true,
                Some(Some(Err(_))) => return false,
                Some(Some(Ok(chunk))) => chunk,
            };
            // An empty chunk would read as the end of a chunked body.
            if chunk.is_empty() {
                continue;
            }
            if self.write(&frame(&chunk)).await.is_err() {
                return false;
            }
        }
    }

    /// Writes and flushes `bytes` within the idle limit. Flushing matters
    /// for TLS, which holds records until flushed.
    async fn write(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        let io = self.io.get_mut();
        let send = async {
            io.write_all(bytes).await?;
            io.flush().await
        };
        timeout(self.idle, send).await.map_err(timed_out)??;
        Ok(())
    }
}

fn timed_out<E>(_: E) -> TransportError {
    TransportError::Io(io::Error::new(io::ErrorKind::TimedOut, "timed out"))
}

/// Sets a header the server owns. Names and values here are fixed or
/// numeric, so they are always valid.
fn set(headers: &mut HeaderMap, name: &str, value: &str) {
    let result = headers.insert(name, value);
    debug_assert!(result.is_ok(), "server header {name}: {value:?}");
}

fn head_bytes(status: StatusCode, headers: &HeaderMap) -> Vec<u8> {
    let head = ResponseHead {
        status,
        reason: status.canonical_reason().unwrap_or("").to_string(),
        version: Version::Http11,
        headers: headers.clone(),
    };
    let mut out = Vec::new();
    head.write(&mut out);
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::convert::Infallible;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use futures_core::Stream;
    use rusty_http::body::response_framing;
    use rusty_http::head::RequestHead;
    use rusty_http::HeaderMap;

    use super::*;
    use crate::response::sse::{Event, Sse};
    use crate::routing::get;

    /// `n` events, each ready at once.
    struct Burst(u32);

    impl Stream for Burst {
        type Item = Result<Event, Infallible>;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if self.0 == 0 {
                return Poll::Ready(None);
            }
            self.0 -= 1;
            Poll::Ready(Some(Ok(Event::default().data("x"))))
        }
    }

    async fn request(conn: &mut AsyncTransport<TcpStream>, target: &str) -> Vec<u8> {
        let mut headers = HeaderMap::new();
        headers.insert("host", "localhost").unwrap();
        let head = RequestHead {
            method: Method::Get,
            target: target.to_string(),
            version: Version::Http11,
            headers,
        };
        conn.write_request_head(&head).await.unwrap();
        let head = conn.read_response_head(1024).await.unwrap();
        let framing = response_framing(&head.headers, &Method::Get, head.status).unwrap();
        conn.read_body(framing).await.unwrap()
    }

    /// Accepts, keep-alive requests and streamed chunks each wait on the
    /// stop signal; none of those waits may outlive itself.
    #[rusty_tokio::test]
    async fn waiting_for_stop_does_not_accumulate() {
        let router = Router::new()
            .route("/", get(|| async { "ok" }))
            .route("/burst", get(|| async { Sse::new(Burst(500)) }));
        let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let server = serve(listener, None, ServeOptions::default(), router).unwrap();
        let connect = || TcpStream::connect_addr(server.local_addr());

        for _ in 0..50 {
            let mut conn = AsyncTransport::new(connect().await.unwrap());
            assert_eq!(request(&mut conn, "/").await, b"ok");
        }
        let mut kept = AsyncTransport::new(connect().await.unwrap());
        for _ in 0..200 {
            assert_eq!(request(&mut kept, "/").await, b"ok");
        }
        for _ in 0..5 {
            assert_eq!(request(&mut kept, "/burst").await.len(), 500 * 9);
        }

        // Left waiting: the accept loop and the one open connection.
        let deadline = Instant::now() + Duration::from_secs(5);
        while server.stop.waiters() > 2 && Instant::now() < deadline {
            sleep(Duration::from_millis(10)).await;
        }
        assert!(
            server.stop.waiters() <= 2,
            "{} waiters",
            server.stop.waiters()
        );
        server.shutdown().await;
    }
}
