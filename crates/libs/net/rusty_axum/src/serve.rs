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
use rusty_tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use rusty_tokio::task::JoinHandle;
use rusty_tokio::time::{sleep, timeout, timeout_at};

use crate::body::Inner;
use crate::request::Request;
use crate::response::Response;
use crate::routing::Router;

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
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Server {
    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stops accepting, closes idle connections, ends streamed bodies, and
    /// waits until requests in flight have been answered.
    pub async fn shutdown(self) {
        // A send fails only when every connection and the accept loop are
        // already gone, which is what is being asked for.
        let _ = self.stop.send(true);
        let _ = self.task.await;
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
    let (stop, stopped) = watch::channel(false);
    let max = options.max_connections.clamp(1, u32::MAX as usize);
    let shared = Arc::new(Shared {
        router,
        tls,
        options,
    });
    Ok(Server {
        local_addr,
        stop,
        task: rusty_tokio::spawn(accept_loop(listener, shared, stopped, max)),
    })
}

struct Shared {
    router: Router,
    tls: Option<TlsAcceptor>,
    options: ServeOptions,
}

/// Waits for `work`, or for the stop signal: `None` once stopped. The stop
/// signal is polled first so that always-ready work cannot starve it.
async fn until_stopped<F: Future>(
    work: F,
    stopped: &mut watch::Receiver<bool>,
) -> Option<F::Output> {
    // Level-triggered as well: a receiver cloned after the signal was sent
    // has already "seen" it and would otherwise never be told.
    if *stopped.borrow() {
        return None;
    }
    rusty_tokio::select! {
        _ = stopped.changed() => None,
        w = work => Some(w),
    }
}

/// Accepts connections until told to stop, then waits for them all to end.
async fn accept_loop(
    listener: TcpListener,
    shared: Arc<Shared>,
    mut stopped: watch::Receiver<bool>,
    max: usize,
) {
    let permits = Arc::new(Semaphore::new(max));
    loop {
        let Some(accepted) = until_stopped(listener.accept(), &mut stopped).await else {
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
        rusty_tokio::spawn(connection(tcp, shared.clone(), stopped.clone(), permit));
    }
    drop(listener);
    // Every connection holds a permit until it ends.
    let _ = permits.acquire_many(max as u32).await;
}

/// One accepted connection: the TLS handshake if any, then HTTP/1.1.
async fn connection(
    tcp: TcpStream,
    shared: Arc<Shared>,
    stopped: watch::Receiver<bool>,
    _permit: OwnedSemaphorePermit,
) {
    let _ = tcp.set_nodelay(true);
    let Some(acceptor) = shared.tls.clone() else {
        return run(tcp, &shared, stopped).await;
    };
    let deadline = Instant::now() + shared.options.handshake_timeout;
    if let Ok(Ok(io)) = timeout_at(deadline, handshake(acceptor, tcp)).await {
        run(io, &shared, stopped).await;
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
async fn run<T>(io: T, shared: &Shared, mut stopped: watch::Receiver<bool>)
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
        let head = match until_stopped(timeout(options.idle_timeout, next), &mut stopped).await {
            Some(Ok(Ok(head))) => head,
            Some(Ok(Err(e))) => return conn.refuse(&e).await,
            // Stopped between requests, or idle too long.
            None | Some(Err(_)) => return,
        };
        let keep_alive = wants_keep_alive(&head);
        let head_only = head.method == Method::Head;
        let req = match conn.read_request(head, options).await {
            Ok(req) => req,
            Err(e) => return conn.refuse(&e).await,
        };
        let response = shared.router.call(req).await;
        let reusable = conn
            .respond(response, head_only, keep_alive, &mut stopped)
            .await;
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
    async fn respond(
        &mut self,
        response: Response,
        head_only: bool,
        keep_alive: bool,
        stopped: &mut watch::Receiver<bool>,
    ) -> bool {
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
                if let Some(length) = &head_length {
                    set(&mut headers, "content-length", length);
                } else if !bodiless {
                    set(&mut headers, "content-length", &bytes.len().to_string());
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
            Inner::Stream(mut stream) => {
                set(&mut headers, "transfer-encoding", "chunked");
                let out = head_bytes(status, &headers);
                if self.write(&out).await.is_err() {
                    return false;
                }
                loop {
                    let next = poll_fn(|cx| stream.as_mut().poll_next(cx));
                    let chunk = match until_stopped(next, stopped).await {
                        // Ended by the stream, or by shutdown: close the
                        // body properly either way.
                        None | Some(None) => {
                            let ended = self.write(b"0\r\n\r\n").await.is_ok();
                            return ended && keep_alive && !*stopped.borrow();
                        }
                        // A failed stream: no terminator, so the client sees
                        // the body truncated, and the connection closes.
                        Some(Some(Err(_))) => return false,
                        Some(Some(Ok(chunk))) => chunk,
                    };
                    // An empty chunk would read as the end of the body.
                    if chunk.is_empty() {
                        continue;
                    }
                    let mut out = Vec::with_capacity(chunk.len() + 16);
                    rusty_http::body::write_chunk(&mut out, &chunk);
                    if self.write(&out).await.is_err() {
                        return false;
                    }
                }
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
