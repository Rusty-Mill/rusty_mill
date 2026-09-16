//! LocalAPI-compatible HTTP-over-UDS server.
//!
//! Serves the subset of tailscaled's LocalAPI that `ts-cli` (and the Go
//! `tailscale` CLI) speaks: `GET /localapi/v0/status`,
//! `PATCH /localapi/v0/prefs`, and `POST /localapi/v0/ping`. The transport is
//! HTTP/1.1 over a Unix domain socket, exactly as the Go daemon does
//! (`ipnlocal`/`localapi`), so an unmodified CLI talks to `ts-daemon`.
//!
//! Ports and adapters: the wire handling lives here; the actual data comes
//! from a [`LocalBackend`] the daemon supplies (backed by the engine). The
//! server never touches the engine directly.
//!
//! HTTP/1.1 framing is `rusty_http`'s job, not `hyper`'s, anymore -- see
//! `DESIGN.md`'s dependency table. `hyper` stays in the dev-dependencies as
//! an independent client for `tests/roundtrip.rs`, proving real HTTP/1.1
//! interop rather than just this crate testing itself.

#![allow(dead_code, unused_imports)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusty_http::head::{RequestHead, ResponseHead};
use rusty_http::tokio_native::AsyncTransport;
use rusty_http::{HeaderMap, Method, StatusCode, Version};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use ts_types::{MaskedPrefs, PingResult, Prefs, Status};

/// Cap on the HTTP/1.1 request head we'll buffer before giving up.
const MAX_HEAD_LEN: usize = 64 * 1024;
/// Cap on a request body (only `PATCH /localapi/v0/prefs` has one, a small
/// JSON masked-prefs edit). A declared `Content-Length` past this is
/// rejected before a single byte of the body is read, so a lying or
/// malicious client can't force us to buffer or wait on an unbounded
/// amount of data.
const MAX_BODY_LEN: u64 = 1 << 20; // 1 MiB

/// The data source behind the LocalAPI: the daemon implements this over the
/// engine handle. Every method is infallible at this layer — errors are
/// encoded in the returned value (e.g. [`PingResult::err`]) as the Go LocalAPI
/// does.
pub trait LocalBackend: Send + Sync + 'static {
    /// `GET /localapi/v0/status`.
    fn status(&self) -> impl std::future::Future<Output = Status> + Send;
    /// `PATCH /localapi/v0/prefs`: apply a masked edit, return the new prefs.
    fn edit_prefs(&self, masked: MaskedPrefs) -> impl std::future::Future<Output = Prefs> + Send;
    /// `POST /localapi/v0/ping?ip=…`: ping a peer, return the result.
    fn ping(&self, ip: std::net::IpAddr) -> impl std::future::Future<Output = PingResult> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("cannot bind LocalAPI socket {path}: {source}")]
    Bind {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("LocalAPI accept error: {0}")]
    Accept(std::io::Error),
}

/// Binds `socket_path` (removing any stale socket first) and serves the
/// LocalAPI until the process exits. One task per connection.
///
/// The socket is created with `0o600` permissions: on Linux the Go daemon
/// authenticates LocalAPI callers by socket peer credentials, and restricting
/// the mode is our first line of defence until we implement the same check.
#[cfg(unix)]
pub async fn serve<B: LocalBackend>(
    socket_path: impl AsRef<Path>,
    backend: B,
) -> Result<(), ServeError> {
    let path = socket_path.as_ref();
    let listener = bind(path)?;
    let backend = Arc::new(backend);
    tracing::info!(socket = %path.display(), "localapi: serving");

    loop {
        let (stream, _addr) = listener.accept().await.map_err(ServeError::Accept)?;
        let backend = backend.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_connection(stream, &*backend).await {
                tracing::debug!("localapi: connection error: {e}");
            }
        });
    }
}

#[cfg(not(unix))]
pub async fn serve<B: LocalBackend>(
    socket_path: impl AsRef<Path>,
    _backend: B,
) -> Result<(), ServeError> {
    Err(ServeError::Bind {
        path: socket_path.as_ref().to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Unix domain sockets are not supported on this platform",
        ),
    })
}

/// Serves every request on one connection until the client closes it or a
/// framing error occurs -- either ends the loop the same way `hyper`'s
/// `serve_connection` did (logged at debug by the caller, not surfaced as a
/// server-level error).
#[cfg(unix)]
async fn serve_connection<B: LocalBackend>(
    stream: UnixStream,
    backend: &B,
) -> rusty_http::TransportResult<()> {
    let mut transport = AsyncTransport::new(stream);
    loop {
        let head = transport.read_request_head(MAX_HEAD_LEN).await?;
        let framing = rusty_http::body::request_framing(&head.headers)?;
        if let rusty_http::body::Framing::ContentLength(len) = framing
            && len > MAX_BODY_LEN
        {
            let (status, content_type, resp_body) = text_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                &format!("request body exceeds {MAX_BODY_LEN} byte limit"),
            );
            write_response(&mut transport, status, content_type, &resp_body).await?;
            // The oversized body was never read off the wire, so the
            // connection can no longer be trusted to be framed correctly
            // for a next request -- close it instead of looping.
            return Ok(());
        }
        let body = transport.read_body(framing).await?;
        let (status, content_type, resp_body) = handle(backend, &head, &body).await;
        write_response(&mut transport, status, content_type, &resp_body).await?;
    }
}

/// Writes a `(status, content_type, body)` triple as a full HTTP/1.1
/// response -- the framing every reply on a connection shares, whether it
/// came from routing a request or from rejecting one up front.
#[cfg(unix)]
async fn write_response(
    transport: &mut AsyncTransport<UnixStream>,
    status: StatusCode,
    content_type: &str,
    body: &[u8],
) -> rusty_http::TransportResult<()> {
    let mut headers = HeaderMap::new();
    let _ = headers.insert("Content-Length", &body.len().to_string());
    let _ = headers.insert("Content-Type", content_type);
    transport
        .write_response_head(&ResponseHead {
            status,
            reason: reason_phrase(status).to_string(),
            version: Version::Http11,
            headers,
        })
        .await?;
    transport.write_body(body).await
}

/// Removes a stale socket, then binds a fresh one with permissions
/// already narrowed to `0600` *before* it starts accepting connections.
///
/// `tokio::net::UnixListener::bind`/`std::os::unix::net::UnixListener::bind`
/// both `bind` *and* `listen` in one call, so a caller that narrows
/// permissions only afterwards (as this function used to) leaves a TOCTOU
/// window: the socket is already connectable, at whatever mode the
/// process umask allows, until the subsequent `chmod` lands.
/// [`unix_bind_chmod_listen`] below instead runs `socket` -> `bind` ->
/// `chmod(0600)` -> `listen`, in that exact order, so the socket cannot
/// accept a single connection until its permissions are already correct.
/// Mirrors rustils' `platform_linux::sys::net::unix_listen` (not taken as
/// a dependency: that crate is Linux-only, while this socket has to bind
/// on every Unix target `ts-daemon` runs on, hence the plain `libc` calls
/// here instead).
#[cfg(unix)]
fn bind(path: &Path) -> Result<UnixListener, ServeError> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        let _ = std::fs::create_dir_all(dir);
    }
    // A leftover socket file from a previous run would make bind() fail with
    // EADDRINUSE even though nothing is listening.
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(ServeError::Bind {
                path: path.to_path_buf(),
                source: e,
            });
        }
    }
    let std_listener =
        unix_bind_chmod_listen(path, narrow_permissions).map_err(|source| ServeError::Bind {
            path: path.to_path_buf(),
            source,
        })?;
    std_listener
        .set_nonblocking(true)
        .map_err(|source| ServeError::Bind {
            path: path.to_path_buf(),
            source,
        })?;
    UnixListener::from_std(std_listener).map_err(|source| ServeError::Bind {
        path: path.to_path_buf(),
        source,
    })
}

/// `socket` + `bind`, deferring `chmod`/`listen` to the caller. Split out
/// from [`unix_bind_chmod_listen`] purely so a test can observe that a
/// freshly bound-but-not-yet-listening socket refuses every connection --
/// the invariant that makes the TOCTOU window structurally impossible,
/// rather than merely narrow.
#[cfg(unix)]
fn unix_socket_bind(path: &Path) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let bytes = path.as_os_str().as_bytes();
    if bytes.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "AF_UNIX path must not contain a NUL byte",
        ));
    }
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    if bytes.len() >= addr.sun_path.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "AF_UNIX path too long (must fit in sockaddr_un::sun_path)",
        ));
    }
    for (slot, byte) in addr.sun_path.iter_mut().zip(bytes.iter()) {
        *slot = *byte as libc::c_char;
    }
    // SAFETY: all-zeroes is a valid (if meaningless) `sockaddr_un`; only
    // the fields set above are ever read back -- the offset below only
    // ever takes the address of a field, never reads through it.
    let base = std::ptr::addr_of!(addr) as usize;
    let sun_path = std::ptr::addr_of!(addr.sun_path) as usize;
    let addr_len = (sun_path - base + bytes.len() + 1) as libc::socklen_t;

    // No `SOCK_CLOEXEC` at `socket(2)`: Darwin has no such type flag, so
    // (matching platform-bsd's own documented portable subset) close-on-
    // exec is set as an explicit second step below instead of atomically.
    // SAFETY: plain integer arguments, no memory referenced.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once so every early return below closes
    // it instead of leaking.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };

    // SAFETY: plain integer arguments, no memory referenced beyond `fd`.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: `addr` holds a valid `sockaddr_un` for exactly `addr_len`
    // bytes constructed above; `fd` is a freshly created, valid socket.
    let r = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&addr as *const libc::sockaddr_un).cast::<libc::sockaddr>(),
            addr_len,
        )
    };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

/// Mode-`0600` (owner read/write only) `chmod` on `path`. This is the
/// step `bind()` used to run *after* the socket had already started
/// listening, and discard the `Result` of (`let _ = set_permissions(..)`)
/// -- a failure here now propagates as a hard error instead of leaving a
/// permanently exposed socket with no error surfaced.
#[cfg(unix)]
fn narrow_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "AF_UNIX path must not contain a NUL byte",
        )
    })?;
    // SAFETY: `c_path` is a valid, NUL-terminated C string outliving the
    // call; `unix_socket_bind` has just created a regular file at this
    // exact path for us to narrow.
    let r = unsafe { libc::chmod(c_path.as_ptr(), 0o600) };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// `listen` on an already-`bind`-ed descriptor, then hand it to `std` as
/// a `UnixListener`. Split out from [`unix_bind_chmod_listen`] for the
/// same testability reason as [`unix_socket_bind`].
#[cfg(unix)]
fn unix_finish_listen(
    fd: std::os::fd::OwnedFd,
) -> std::io::Result<std::os::unix::net::UnixListener> {
    use std::os::fd::AsRawFd;
    // SAFETY: `fd` is a valid, bound socket.
    let r = unsafe { libc::listen(fd.as_raw_fd(), libc::SOMAXCONN) };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(std::os::unix::net::UnixListener::from(fd))
}

/// `unix_socket_bind` -> `chmod` -> `unix_finish_listen`, in that exact
/// order: the socket cannot accept a single connection until `chmod` has
/// already succeeded. `chmod` is threaded through as a parameter (rather
/// than calling [`narrow_permissions`] inline) solely so a test can swap
/// in a failing stand-in and prove a `chmod` failure surfaces as `Err`
/// from this exact sequence -- the one `bind()` composes in production --
/// instead of being silently swallowed.
#[cfg(unix)]
fn unix_bind_chmod_listen(
    path: &Path,
    chmod: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<std::os::unix::net::UnixListener> {
    let fd = unix_socket_bind(path)?;
    chmod(path)?;
    unix_finish_listen(fd)
}

/// A canonical reason phrase for the status codes this server ever sends --
/// unlike `hyper`, `rusty_http::head::ResponseHead` takes the caller's
/// choice of reason phrase rather than deriving one from the status code.
fn reason_phrase(status: StatusCode) -> &'static str {
    match status.as_u16() {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "",
    }
}

fn json_response(status: StatusCode, body: Vec<u8>) -> (StatusCode, &'static str, Vec<u8>) {
    (status, "application/json", body)
}

fn text_error(status: StatusCode, msg: &str) -> (StatusCode, &'static str, Vec<u8>) {
    (
        status,
        "text/plain; charset=utf-8",
        format!("{msg}\n").into_bytes(),
    )
}

/// Splits a request-target into (path, query), the same split
/// `req.uri().path()`/`.query()` gave for free under `hyper`.
fn split_target(target: &str) -> (&str, &str) {
    match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    }
}

/// Routes one request to a backend method and encodes the response.
async fn handle<B: LocalBackend>(
    backend: &B,
    head: &RequestHead,
    body: &[u8],
) -> (StatusCode, &'static str, Vec<u8>) {
    let (path, query) = split_target(&head.target);

    match (&head.method, path) {
        (Method::Get, "/localapi/v0/status") => {
            let status = backend.status().await;
            encode_json(&status)
        }
        (Method::Patch, "/localapi/v0/prefs") => {
            let masked: MaskedPrefs = match serde_json::from_slice(body) {
                Ok(m) => m,
                Err(e) => {
                    return text_error(StatusCode::from_u16(400), &format!("bad prefs: {e}"));
                }
            };
            let prefs = backend.edit_prefs(masked).await;
            encode_json(&prefs)
        }
        (Method::Post, "/localapi/v0/ping") => {
            let Some(ip) = query_param(query, "ip") else {
                return text_error(StatusCode::from_u16(400), "ping requires an ip parameter");
            };
            let Ok(ip) = ip.parse::<std::net::IpAddr>() else {
                return text_error(StatusCode::from_u16(400), "invalid ip parameter");
            };
            let result = backend.ping(ip).await;
            encode_json(&result)
        }
        _ => text_error(StatusCode::from_u16(404), "not found"),
    }
}

fn encode_json<T: serde::Serialize>(value: &T) -> (StatusCode, &'static str, Vec<u8>) {
    match serde_json::to_vec(value) {
        Ok(bytes) => json_response(StatusCode::from_u16(200), bytes),
        Err(e) => text_error(StatusCode::from_u16(500), &format!("encode error: {e}")),
    }
}

/// Extracts a value from a raw `k=v&k2=v2` query string (minimal; the CLI only
/// sends already-safe ASCII IP/type parameters, so no percent-decoding).
fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then_some(v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_param_parses() {
        assert_eq!(
            query_param("ip=100.64.0.2&type=disco", "ip"),
            Some("100.64.0.2")
        );
        assert_eq!(
            query_param("ip=100.64.0.2&type=disco", "type"),
            Some("disco")
        );
        assert_eq!(query_param("ip=100.64.0.2", "missing"), None);
        assert_eq!(query_param("", "ip"), None);
    }

    /// Round 5: `serve_connection` used to call `read_body` with no cap on
    /// a declared `Content-Length`, so a PATCH claiming a huge body (that
    /// never actually arrives) would hang the connection forever waiting
    /// for bytes to read. The cap must reject it up front with a 413,
    /// without ever trying to read the body.
    #[cfg(unix)]
    #[tokio::test]
    async fn oversized_content_length_is_rejected_before_reading_the_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        struct EmptyBackend;
        impl LocalBackend for EmptyBackend {
            async fn status(&self) -> Status {
                Status::default()
            }
            async fn edit_prefs(&self, _masked: MaskedPrefs) -> Prefs {
                Prefs::default()
            }
            async fn ping(&self, _ip: std::net::IpAddr) -> PingResult {
                PingResult::default()
            }
        }

        let (mut client, server) = UnixStream::pair().unwrap();
        let server_task = tokio::spawn(async move {
            let backend = EmptyBackend;
            let _ = serve_connection(server, &backend).await;
        });

        // A PATCH declaring a body far past the cap; the body itself is
        // never sent. If the cap weren't enforced up front, `read_body`
        // would block forever waiting for bytes that never arrive.
        let request = format!(
            "PATCH /localapi/v0/prefs HTTP/1.1\r\nHost: local\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_LEN + 1
        );
        client.write_all(request.as_bytes()).await.unwrap();

        let mut response = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.read_to_end(&mut response),
        )
        .await
        .expect("server must respond (and close) instead of hanging on the oversized body")
        .unwrap();

        let response = String::from_utf8(response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 413"),
            "expected a 413 Payload Too Large, got: {response}"
        );

        server_task.await.unwrap();
    }

    /// A fresh, unique scratch directory under the OS temp dir -- one per
    /// test invocation (mixes in the PID and a per-call counter, since
    /// several of these tests run concurrently within the same process).
    #[cfg(unix)]
    fn unique_test_dir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ts-localapi-bind-test-{label}-{}-{n}",
            std::process::id()
        ))
    }

    /// Regression for the bind-then-chmod TOCTOU window: `bind()` used to
    /// call `UnixListener::bind` (which binds *and* starts listening in
    /// one step) and only narrowed permissions afterwards, so a
    /// freshly-listening socket sat at the ambient umask's permissions
    /// until the subsequent `chmod` landed. The fix reorders this to
    /// `bind` -> `chmod` -> `listen`, which this test proves structurally:
    /// a socket that has been `bind`-ed but not yet had `chmod`/`listen`
    /// run on it must refuse every connection, so there is no window in
    /// which it is both loosely permissioned *and* acceptable.
    #[cfg(unix)]
    #[test]
    fn socket_cannot_accept_connections_before_permissions_are_narrowed() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_test_dir("no-window");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.sock");

        let fd = unix_socket_bind(&path).expect("socket+bind must succeed");

        // The socket *file* already exists on disk (bind() alone creates
        // it), but nothing is listening yet -- a connect attempt must
        // fail. This is the invariant that makes the old TOCTOU window
        // structurally impossible: whatever the file's mode is at this
        // point, it cannot matter, because nothing can connect regardless.
        let refused = std::os::unix::net::UnixStream::connect(&path);
        assert!(
            refused.is_err(),
            "a bound-but-not-yet-listening socket must refuse connections, not accept one"
        );

        narrow_permissions(&path).expect("chmod must succeed on the just-bound socket");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "permissions must already be narrowed before listen() runs"
        );

        let listener = unix_finish_listen(fd).expect("listen must succeed");
        let accepted = std::thread::spawn(move || listener.accept());
        let client = std::os::unix::net::UnixStream::connect(&path).expect("connect after listen");
        drop(client);
        accepted
            .join()
            .unwrap()
            .expect("listener must accept once listen() has actually run");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real `chmod(2)` wrapper `bind()` composes with must be able to
    /// report a genuine OS failure, not just theoretically -- proven here
    /// with a real, deterministic `ENOENT` (no such path) rather than a
    /// synthetic error.
    #[cfg(unix)]
    #[test]
    fn narrow_permissions_reports_a_real_chmod_failure() {
        let dir = unique_test_dir("real-chmod-failure");
        let path = dir.join("missing.sock");
        let _ = std::fs::remove_dir_all(&dir);

        let err = narrow_permissions(&path).expect_err("chmod on a nonexistent path must fail");
        assert_eq!(err.raw_os_error(), Some(libc::ENOENT));
    }

    /// Regression for the discarded `let _ = set_permissions(..)`: a
    /// `chmod` failure inside the bind-chmod-listen sequence must
    /// propagate as `Err` from that sequence (the exact one `bind()`
    /// composes with `narrow_permissions` in production), not be
    /// swallowed and let the caller carry on to `listen()` regardless.
    #[cfg(unix)]
    #[test]
    fn chmod_failure_surfaces_as_an_error_instead_of_being_swallowed() {
        let dir = unique_test_dir("swallowed-chmod-failure");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.sock");

        let err = unix_bind_chmod_listen(&path, |_path| {
            Err(std::io::Error::from_raw_os_error(libc::EACCES))
        })
        .expect_err("a chmod failure inside bind()'s sequence must surface as Err");
        assert_eq!(err.raw_os_error(), Some(libc::EACCES));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
