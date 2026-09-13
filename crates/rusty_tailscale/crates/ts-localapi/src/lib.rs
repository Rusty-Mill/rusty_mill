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

/// Removes a stale socket, binds a fresh one, and tightens its permissions.
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
    let listener = UnixListener::bind(path).map_err(|source| ServeError::Bind {
        path: path.to_path_buf(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(listener)
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
}
