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

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::UnixListener;
use ts_types::{MaskedPrefs, PingResult, Prefs, Status};

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
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| {
                let backend = backend.clone();
                async move { Ok::<_, Infallible>(handle(&*backend, req).await) }
            });
            if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                tracing::debug!("localapi: connection error: {e}");
            }
        });
    }
}

/// Removes a stale socket, binds a fresh one, and tightens its permissions.
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

type Body = Full<Bytes>;

fn json_response(status: StatusCode, body: Bytes) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Full::new(body))
        .expect("static response is valid")
}

fn text_error(status: StatusCode, msg: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(format!("{msg}\n"))))
        .expect("static response is valid")
}

/// Routes one request to a backend method and encodes the response.
async fn handle<B: LocalBackend>(
    backend: &B,
    req: Request<hyper::body::Incoming>,
) -> Response<Body> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

    match (&method, path.as_str()) {
        (&Method::GET, "/localapi/v0/status") => {
            let status = backend.status().await;
            encode_json(&status)
        }
        (&Method::PATCH, "/localapi/v0/prefs") => {
            let body = match collect_body(req).await {
                Ok(b) => b,
                Err(resp) => return resp,
            };
            let masked: MaskedPrefs = match serde_json::from_slice(&body) {
                Ok(m) => m,
                Err(e) => return text_error(StatusCode::BAD_REQUEST, &format!("bad prefs: {e}")),
            };
            let prefs = backend.edit_prefs(masked).await;
            encode_json(&prefs)
        }
        (&Method::POST, "/localapi/v0/ping") => {
            let Some(ip) = query_param(&query, "ip") else {
                return text_error(StatusCode::BAD_REQUEST, "ping requires an ip parameter");
            };
            let Ok(ip) = ip.parse::<std::net::IpAddr>() else {
                return text_error(StatusCode::BAD_REQUEST, "invalid ip parameter");
            };
            let result = backend.ping(ip).await;
            encode_json(&result)
        }
        _ => text_error(StatusCode::NOT_FOUND, "not found"),
    }
}

fn encode_json<T: serde::Serialize>(value: &T) -> Response<Body> {
    match serde_json::to_vec(value) {
        Ok(bytes) => json_response(StatusCode::OK, Bytes::from(bytes)),
        Err(e) => text_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("encode error: {e}"),
        ),
    }
}

async fn collect_body(req: Request<hyper::body::Incoming>) -> Result<Bytes, Response<Body>> {
    req.into_body()
        .collect()
        .await
        .map(|c| c.to_bytes())
        .map_err(|e| text_error(StatusCode::BAD_REQUEST, &format!("read body: {e}")))
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
}
