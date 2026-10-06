//! A small blocking HTTP/1.1 server for a JSON API plus a built web UI,
//! on `rusty_http`'s framing: `std::net` sockets, one thread per
//! connection, no async runtime. Extracted from `rusty_tick`'s `server.rs`
//! when `rusty_fair_play` needed the same file (its ADR-0001), so the two
//! apps share one copy.
//!
//! The server owns the socket and nothing else: a [`Handler`] answers one
//! parsed [`Request`] with one [`Response`] and never sees a byte of the
//! transport, so every route is testable without a network. Chosen over
//! an async runtime deliberately: a personal server has a handful of
//! concurrent connections and one store behind one lock, so there is no
//! I/O concurrency for a runtime to exploit. Bounds on head size, body
//! size, idle time and connection count keep a slow or hostile client
//! from holding resources.
//!
//! Paths outside `/api` and `/health` serve the built web UI from a
//! directory when one is set ([`Server::with_web_dir`]), with a strict
//! `Content-Security-Policy`; see [`static_files`] for the path rules.

pub mod static_files;

use rusty_http::body::{request_framing, Framing};
use rusty_http::head::ResponseHead;
use rusty_http::sync::SyncTransport;
use rusty_http::{Method, StatusCode, TransportError, TransportResult, Version};

/// Re-exported so a handler can name the header type without depending on
/// `rusty_http` itself.
pub use rusty_http::HeaderMap;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_HEAD_BYTES: usize = 16 * 1024;
/// Largest request body accepted.
pub const MAX_BODY_BYTES: u64 = 1024 * 1024;
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_MAX_CONNECTIONS: usize = 64;

/// A parsed request, as a [`Handler`] sees it.
pub struct Request<'a> {
    pub method: &'a Method,
    /// Origin-form target: path and optional `?query`.
    pub target: &'a str,
    pub authorization: Option<&'a str>,
    pub if_match: Option<&'a str>,
    /// Every header, for a handler that needs more than the two above (a
    /// webhook's signature, say).
    pub headers: &'a HeaderMap,
    pub body: &'a [u8],
}

/// What a [`Handler`] answers: a status and a body.
pub struct Response {
    pub status: StatusCode,
    pub body: Body,
}

/// A response body: a JSON document written whole, or a stream of chunks
/// written as they are produced.
pub enum Body {
    /// A JSON document, empty for 204. Sent with `Content-Length`.
    Json(Vec<u8>),
    /// Chunks pulled from the iterator after the handler has returned and
    /// its lock is released, each written to the socket as one
    /// `Transfer-Encoding: chunked` chunk, until the iterator ends. For
    /// `text/event-stream` and other long-lived responses.
    Stream {
        /// The `Content-Type` to send.
        content_type: &'static str,
        /// The chunks. Blocking in `next` is fine: the connection thread
        /// is the only one waiting.
        chunks: Box<dyn Iterator<Item = Vec<u8>> + Send>,
    },
    /// Work that produces the real response, run after the handler has
    /// returned and its lock is released, on the connection's own thread. For
    /// a route that waits on something slow (another server, say) and so must
    /// not hold up every other request. The [`Response::status`] it is built
    /// with is a placeholder; the job's own status is what is sent.
    Deferred(Job),
}

/// The work of a [`Body::Deferred`]: the status and JSON body to send.
pub type Job = Box<dyn FnOnce() -> (StatusCode, Vec<u8>) + Send>;

impl Response {
    /// A JSON response (an empty body for 204).
    pub fn json(status: StatusCode, body: Vec<u8>) -> Self {
        Self {
            status,
            body: Body::Json(body),
        }
    }

    /// A streamed response; see [`Body::Stream`].
    pub fn stream(
        content_type: &'static str,
        chunks: impl Iterator<Item = Vec<u8>> + Send + 'static,
    ) -> Self {
        Self {
            status: StatusCode::OK,
            body: Body::Stream {
                content_type,
                chunks: Box::new(chunks),
            },
        }
    }

    /// A response computed by `job` after the handler's lock is released; see
    /// [`Body::Deferred`]. The job gets no access to the handler, so copy
    /// whatever it needs out of the request first.
    pub fn deferred(job: impl FnOnce() -> (StatusCode, Vec<u8>) + Send + 'static) -> Self {
        Self {
            status: StatusCode::OK,
            body: Body::Deferred(Box::new(job)),
        }
    }

    /// A generic 500 in the error shape every handler here uses,
    /// `{"error":{"code":"internal","message":"internal error"}}` — what
    /// the transport answers when the handler is unusable (its lock is
    /// poisoned, so a handler panicked mid-write).
    pub fn internal_error() -> Self {
        Self::json(
            StatusCode::INTERNAL_SERVER_ERROR,
            br#"{"error":{"code":"internal","message":"internal error"}}"#.to_vec(),
        )
    }
}

/// The application behind the socket: one request in, one response out.
/// Called under one lock, so a handler may hold `&mut self` state.
pub trait Handler: Send {
    fn handle(&mut self, request: &Request<'_>) -> Response;
}

struct App<H> {
    /// One lock around the handler and whatever it owns.
    handler: Mutex<H>,
    /// Built web UI to serve at `/`, if any.
    web_dir: Option<PathBuf>,
}

pub struct Server<H> {
    listener: TcpListener,
    app: Arc<App<H>>,
    stop: Arc<AtomicBool>,
    max_connections: usize,
}

/// Stops a running [`Server`]; cloneable and usable from another thread.
#[derive(Clone)]
pub struct ShutdownHandle {
    stop: Arc<AtomicBool>,
    addr: SocketAddr,
}

impl ShutdownHandle {
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocking `accept` so the loop sees the flag.
        let _ = TcpStream::connect(self.addr);
    }
}

impl<H: Handler + 'static> Server<H> {
    pub fn bind(addr: SocketAddr, handler: H) -> io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr)?,
            app: Arc::new(App {
                handler: Mutex::new(handler),
                web_dir: None,
            }),
            stop: Arc::new(AtomicBool::new(false)),
            max_connections: DEFAULT_MAX_CONNECTIONS,
        })
    }

    /// Serve the built web UI in `dir` for every `GET` outside `/api` and
    /// `/health`.
    #[must_use]
    pub fn with_web_dir(mut self, dir: PathBuf) -> Self {
        // No other handle exists before `run`, so this never clones.
        if let Some(app) = Arc::get_mut(&mut self.app) {
            app.web_dir = Some(dir);
        }
        self
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn shutdown_handle(&self) -> io::Result<ShutdownHandle> {
        Ok(ShutdownHandle {
            stop: Arc::clone(&self.stop),
            addr: self.local_addr()?,
        })
    }

    /// Accept connections until [`ShutdownHandle::shutdown`].
    pub fn run(self) -> io::Result<()> {
        let active = Arc::new(AtomicUsize::new(0));
        for stream in self.listener.incoming() {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else { continue };
            if active.load(Ordering::SeqCst) >= self.max_connections {
                refuse(stream);
                continue;
            }
            active.fetch_add(1, Ordering::SeqCst);
            let (app, active) = (Arc::clone(&self.app), Arc::clone(&active));
            std::thread::spawn(move || {
                let _ = serve_connection(stream, &app);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
        Ok(())
    }
}

/// Best-effort 503 to a connection over the limit.
fn refuse(stream: TcpStream) {
    let mut transport = SyncTransport::new(stream);
    let _ = write_response(&mut transport, StatusCode::SERVICE_UNAVAILABLE, b"", false);
}

fn serve_connection<H: Handler>(stream: TcpStream, app: &App<H>) -> TransportResult<()> {
    stream.set_read_timeout(Some(IDLE_TIMEOUT))?;
    stream.set_write_timeout(Some(IDLE_TIMEOUT))?;
    let mut transport = SyncTransport::new(stream);
    loop {
        let head = match transport.read_request_head(MAX_HEAD_BYTES) {
            Ok(head) => head,
            // A malformed head gets an answer; a closed or timed-out
            // connection just ends.
            Err(TransportError::Http(_)) => {
                return write_response(&mut transport, StatusCode::BAD_REQUEST, b"", false);
            }
            Err(e) => return Err(e),
        };
        let body = match request_framing(&head.headers) {
            Ok(Framing::None) => Vec::new(),
            Ok(Framing::ContentLength(len)) if len <= MAX_BODY_BYTES => {
                transport.read_content_length_body(len, MAX_BODY_BYTES)?
            }
            Ok(Framing::ContentLength(_)) => {
                return write_response(&mut transport, StatusCode::PAYLOAD_TOO_LARGE, b"", false);
            }
            // Chunked request bodies are not supported; clients send a length.
            Ok(_) => {
                return write_response(&mut transport, StatusCode::LENGTH_REQUIRED, b"", false);
            }
            Err(_) => {
                return write_response(&mut transport, StatusCode::BAD_REQUEST, b"", false);
            }
        };
        let keep_alive = wants_keep_alive(&head.headers, head.version);
        if let Some(asset) = static_asset(app, &head.method, &head.target) {
            write_asset(&mut transport, &asset, keep_alive)?;
            if !keep_alive {
                return Ok(());
            }
            continue;
        }
        let request = Request {
            method: &head.method,
            target: &head.target,
            authorization: head.headers.get("authorization"),
            if_match: head.headers.get("if-match"),
            headers: &head.headers,
            body: &body,
        };
        let response = match app.handler.lock() {
            Ok(mut handler) => handler.handle(&request),
            // A poisoned lock means a handler panicked mid-write; refuse
            // rather than serve state that may be inconsistent.
            Err(_) => Response::internal_error(),
        };
        match response.body {
            Body::Json(body) => write_response(&mut transport, response.status, &body, keep_alive)?,
            Body::Deferred(job) => {
                let (status, body) = job();
                write_response(&mut transport, status, &body, keep_alive)?;
            }
            Body::Stream {
                content_type,
                chunks,
            } => write_stream(
                &mut transport,
                response.status,
                content_type,
                chunks,
                keep_alive,
            )?,
        }
        if !keep_alive {
            return Ok(());
        }
    }
}

/// Send a streamed body as chunked transfer encoding, one chunk per
/// iterator item, ending the body when the iterator does. Nothing is
/// buffered: each chunk reaches the socket before the next is pulled, so
/// a client sees events as the handler's producer emits them.
fn write_stream(
    transport: &mut SyncTransport<TcpStream>,
    status: StatusCode,
    content_type: &'static str,
    chunks: Box<dyn Iterator<Item = Vec<u8>> + Send>,
    keep_alive: bool,
) -> TransportResult<()> {
    let mut headers = HeaderMap::new();
    let _ = headers.insert("Content-Type", content_type);
    let _ = headers.insert("Transfer-Encoding", "chunked");
    let _ = headers.insert("Cache-Control", "no-store");
    let _ = headers.insert("X-Content-Type-Options", "nosniff");
    if !keep_alive {
        let _ = headers.insert("Connection", "close");
    }
    transport.write_response_head(&ResponseHead {
        status,
        reason: status.canonical_reason().unwrap_or("").to_string(),
        version: Version::Http11,
        headers,
    })?;
    for chunk in chunks {
        transport.write_chunk(&chunk)?;
    }
    transport.write_chunked_end()
}

/// The web UI file for a `GET` outside the API, when a UI directory is set.
fn static_asset<H>(app: &App<H>, method: &Method, target: &str) -> Option<static_files::Asset> {
    let root = app.web_dir.as_ref()?;
    if method != &Method::Get {
        return None;
    }
    let path = target.split('?').next().unwrap_or("");
    if path == "/health" || path == "/api" || path.starts_with("/api/") {
        return None;
    }
    static_files::load(root, path)
}

fn wants_keep_alive(headers: &HeaderMap, version: Version) -> bool {
    let connection = headers.get("connection").unwrap_or("");
    if connection.eq_ignore_ascii_case("close") {
        return false;
    }
    version == Version::Http11 || connection.eq_ignore_ascii_case("keep-alive")
}

fn write_response(
    transport: &mut SyncTransport<TcpStream>,
    status: StatusCode,
    body: &[u8],
    keep_alive: bool,
) -> TransportResult<()> {
    let mut headers = HeaderMap::new();
    if status != StatusCode::NO_CONTENT {
        let _ = headers.insert("Content-Length", &body.len().to_string());
    }
    if !body.is_empty() {
        let _ = headers.insert("Content-Type", "application/json");
    }
    if status == StatusCode::UNAUTHORIZED {
        let _ = headers.insert("WWW-Authenticate", "Bearer");
    }
    let _ = headers.insert("Cache-Control", "no-store");
    let _ = headers.insert("X-Content-Type-Options", "nosniff");
    if !keep_alive {
        let _ = headers.insert("Connection", "close");
    }
    transport.write_response_head(&ResponseHead {
        status,
        reason: status.canonical_reason().unwrap_or("").to_string(),
        version: Version::Http11,
        headers,
    })?;
    if !body.is_empty() {
        transport.write_body(body)?;
    }
    Ok(())
}

/// Send a static asset. Pages get a strict CSP; nothing is cached so a
/// rebuilt UI shows up on reload.
fn write_asset(
    transport: &mut SyncTransport<TcpStream>,
    asset: &static_files::Asset,
    keep_alive: bool,
) -> TransportResult<()> {
    let mut headers = HeaderMap::new();
    let _ = headers.insert("Content-Length", &asset.body.len().to_string());
    let _ = headers.insert("Content-Type", asset.content_type);
    let _ = headers.insert("Cache-Control", "no-cache");
    let _ = headers.insert("X-Content-Type-Options", "nosniff");
    if asset.content_type.starts_with("text/html") {
        let _ = headers.insert("Content-Security-Policy", CSP);
        let _ = headers.insert("X-Frame-Options", "DENY");
        let _ = headers.insert("Referrer-Policy", "no-referrer");
    }
    if !keep_alive {
        let _ = headers.insert("Connection", "close");
    }
    transport.write_response_head(&ResponseHead {
        status: StatusCode::OK,
        reason: "OK".to_string(),
        version: Version::Http11,
        headers,
    })?;
    transport.write_body(&asset.body)
}

/// Scripts and styles only from this origin; the UI talks only to this
/// origin's API. Inline styles are allowed because a UI sets colours from
/// data (a list's, a suit's) through the `style` attribute.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Held shut until a test opens it; see `/api/slow`.
    static GATE: std::sync::LazyLock<(Mutex<bool>, std::sync::Condvar)> =
        std::sync::LazyLock::new(|| (Mutex::new(false), std::sync::Condvar::new()));

    /// Echoes the method and target as JSON; `/boom` panics so the lock
    /// poisons.
    struct Echo;

    impl Handler for Echo {
        fn handle(&mut self, request: &Request<'_>) -> Response {
            assert!(request.target != "/api/boom", "poison the lock");
            if request.target == "/api/slow" {
                // Answered only once the test opens the gate, and not under the lock.
                return Response::deferred(|| {
                    let (open, opened) = &*GATE;
                    let _guard = opened
                        .wait_while(open.lock().unwrap(), |open| !*open)
                        .unwrap();
                    (StatusCode::ACCEPTED, br#"{"slow":true}"#.to_vec())
                });
            }
            if request.target == "/api/stream" {
                // Three chunks, the second produced after the first was sent.
                return Response::stream(
                    "text/event-stream",
                    (1..=3).map(|n| format!("data: {n}\n\n").into_bytes()),
                );
            }
            Response::json(
                StatusCode::OK,
                format!(
                    r#"{{"method":"{:?}","target":"{}","ifMatch":{}}}"#,
                    request.method,
                    request.target,
                    request
                        .if_match
                        .map_or("null".to_string(), |v| format!("\"{v}\""))
                )
                .into_bytes(),
            )
        }
    }

    fn start(web_dir: Option<PathBuf>) -> (SocketAddr, ShutdownHandle) {
        let mut server = Server::bind("127.0.0.1:0".parse().unwrap(), Echo).unwrap();
        if let Some(dir) = web_dir {
            server = server.with_web_dir(dir);
        }
        let addr = server.local_addr().unwrap();
        let stop = server.shutdown_handle().unwrap();
        std::thread::spawn(move || server.run().unwrap());
        (addr, stop)
    }

    fn exchange(addr: SocketAddr, raw: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    }

    fn status(response: &str) -> u16 {
        response.split(' ').nth(1).unwrap().parse().unwrap()
    }

    #[test]
    fn routes_api_to_the_handler_and_the_rest_to_the_ui() {
        let web = tempfile::tempdir().unwrap();
        std::fs::write(web.path().join("index.html"), "<html>ui</html>").unwrap();
        let (addr, stop) = start(Some(web.path().to_path_buf()));

        let api = exchange(
            addr,
            "GET /api/v1/x?y=1 HTTP/1.1\r\nHost: t\r\nConnection: close\r\nIf-Match: \"7\"\r\n\r\n",
        );
        assert_eq!(status(&api), 200);
        assert!(api.contains(r#""target":"/api/v1/x?y=1""#));
        assert!(
            api.contains(r#""ifMatch":""7"""#),
            "the header is passed through verbatim, quotes included"
        );
        assert!(api.contains("Content-Type: application/json"));

        for path in ["/", "/deck/abc", "/assets/missing.js"] {
            let page = exchange(
                addr,
                &format!("GET {path} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n"),
            );
            assert_eq!(status(&page), 200, "{path}");
            assert!(page.contains("Content-Security-Policy"), "{path}");
            assert!(page.ends_with("<html>ui</html>"), "{path}");
        }
        // A non-GET outside the API still reaches the handler.
        let post = exchange(
            addr,
            "POST /anything HTTP/1.1\r\nHost: t\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        );
        assert!(post.contains(r#""method":"Post""#));
        stop.shutdown();
    }

    #[test]
    fn limits_and_malformed_input_are_answered_not_served() {
        let (addr, stop) = start(None);
        let too_big = exchange(
            addr,
            &format!(
                "POST /api HTTP/1.1\r\nHost: t\r\nContent-Length: {}\r\n\r\n",
                MAX_BODY_BYTES + 1
            ),
        );
        assert_eq!(status(&too_big), 413);
        let chunked = exchange(
            addr,
            "POST /api HTTP/1.1\r\nHost: t\r\nTransfer-Encoding: chunked\r\n\r\n",
        );
        assert_eq!(status(&chunked), 411);
        let garbage = exchange(addr, "not http\r\n\r\n");
        assert_eq!(status(&garbage), 400);
        // A handler panic poisons the lock; later requests get a 500, not a hang.
        let boom = exchange(
            addr,
            "GET /api/boom HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
        );
        assert!(
            boom.is_empty() || status(&boom) >= 500,
            "the panicking connection ends"
        );
        let after = exchange(
            addr,
            "GET /api/v1/x HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status(&after), 500);
        assert!(after.contains(r#""code":"internal""#));
        stop.shutdown();
    }

    #[test]
    fn a_streamed_body_is_chunked_and_keeps_the_connection_usable() {
        let (addr, stop) = start(None);
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .write_all(b"GET /api/stream HTTP/1.1\r\nHost: t\r\n\r\n")
            .unwrap();
        stream
            .write_all(b"GET /api/after HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        let (first, second) = out.split_once("HTTP/1.1 200 OK\r\nContent-Length").unwrap();
        assert!(
            first.contains("Content-Type: text/event-stream\r\n"),
            "{first}"
        );
        assert!(first.contains("Transfer-Encoding: chunked\r\n"), "{first}");
        assert!(!first.contains("Content-Length"), "{first}");
        assert!(
            first
                .ends_with("9\r\ndata: 1\n\n\r\n9\r\ndata: 2\n\n\r\n9\r\ndata: 3\n\n\r\n0\r\n\r\n"),
            "{first:?}"
        );
        assert!(
            second.contains("/api/after"),
            "keep-alive survives a stream"
        );
        stop.shutdown();
    }

    #[test]
    fn a_deferred_response_runs_without_the_lock_so_other_requests_are_not_held_up() {
        let (addr, stop) = start(None);
        let slow = std::thread::spawn(move || {
            exchange(
                addr,
                "GET /api/slow HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
            )
        });
        // Give the slow request time to reach its job and start waiting.
        std::thread::sleep(Duration::from_millis(300));
        let fast = exchange(
            addr,
            "GET /api/fast HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status(&fast), 200, "answered while the slow one waits");
        assert!(!slow.is_finished(), "the slow one is still waiting");

        let (open, opened) = &*GATE;
        *open.lock().unwrap() = true;
        opened.notify_all();
        let slow = slow.join().unwrap();
        assert_eq!(status(&slow), 202, "the job's own status is what is sent");
        assert!(slow.ends_with(r#"{"slow":true}"#), "{slow}");
        stop.shutdown();
    }

    #[test]
    fn keep_alive_serves_two_requests_on_one_connection() {
        let (addr, stop) = start(None);
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let one = "GET /api/one HTTP/1.1\r\nHost: t\r\n\r\n";
        let two = "GET /api/two HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n";
        stream.write_all(one.as_bytes()).unwrap();
        stream.write_all(two.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        assert_eq!(out.matches("HTTP/1.1 200").count(), 2);
        assert!(out.contains("/api/one") && out.contains("/api/two"));
        stop.shutdown();
    }
}
