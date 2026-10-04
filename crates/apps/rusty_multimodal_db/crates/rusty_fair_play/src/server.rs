//! The TCP adapter: `std::net` sockets, one thread per connection, HTTP/1.1
//! framing by `rusty_http`. Everything about the API itself lives in
//! [`crate::api`]; this file only moves bytes.
//!
//! `rusty_tick`'s `server.rs`, with its backend replaced by [`Api`] (one
//! service, one optional token). Chosen over an async runtime deliberately:
//! a household has a handful of concurrent connections and one store
//! behind one lock, so there is no I/O concurrency for a runtime to
//! exploit. Bounds on head size, body size, idle time and connection count
//! keep a slow or hostile client from holding resources. The second copy
//! of this file in the workspace: a shared `rusty_http` sync server is the
//! dedupe candidate once a third appears.

use crate::api::{Api, Request};
use crate::static_files;
use rusty_http::body::{request_framing, Framing};
use rusty_http::head::ResponseHead;
use rusty_http::sync::SyncTransport;
use rusty_http::{HeaderMap, Method, StatusCode, TransportError, TransportResult, Version};
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_HEAD_BYTES: usize = 16 * 1024;
/// Largest request body accepted; notes are capped well below this.
pub const MAX_BODY_BYTES: u64 = 1024 * 1024;
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_MAX_CONNECTIONS: usize = 64;

struct App {
    /// One lock around the token check and the store.
    api: Mutex<Api>,
    /// Built web UI to serve at `/`, if any.
    web_dir: Option<PathBuf>,
}

pub struct Server {
    listener: TcpListener,
    app: Arc<App>,
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

impl Server {
    pub fn bind(addr: SocketAddr, api: Api) -> io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr)?,
            app: Arc::new(App {
                api: Mutex::new(api),
                web_dir: None,
            }),
            stop: Arc::new(AtomicBool::new(false)),
            max_connections: DEFAULT_MAX_CONNECTIONS,
        })
    }

    /// Serve the built web UI in `dir` for every path outside `/api` and `/health`.
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

fn serve_connection(stream: TcpStream, app: &App) -> TransportResult<()> {
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
            body: &body,
        };
        let response = match app.api.lock() {
            Ok(mut api) => api.handle(&request),
            Err(_) => internal_error(),
        };
        write_response(&mut transport, response.status, &response.body, keep_alive)?;
        if !keep_alive {
            return Ok(());
        }
    }
}

/// The web UI file for a `GET` outside the API, when a UI directory is set.
fn static_asset(app: &App, method: &Method, target: &str) -> Option<static_files::Asset> {
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

/// A poisoned lock means a handler panicked mid-write; refuse rather than
/// serve state that may be inconsistent.
fn internal_error() -> crate::api::Response {
    crate::api::Response::internal_error()
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
/// origin's API. Inline styles are allowed because the UI sets suit colours
/// through the `style` attribute.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";
