//! Minimal LocalAPI client: HTTP/1.1 over the tailscaled Unix socket.
//!
//! Mirrors the Go client (`client/tailscale/localclient.go`): requests target
//! `http://local-tailscaled.sock/localapi/v0/...` with the `Host` header set
//! to `local-tailscaled.sock`; on Linux authentication is by socket peer
//! credentials. One connection per request — a CLI has no use for a pool.

use std::path::PathBuf;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, StatusCode, header};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;

/// Default tailscaled socket path on Linux.
pub const DEFAULT_SOCKET: &str = "/var/run/tailscale/tailscaled.sock";

const HOST: &str = "local-tailscaled.sock";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot connect to tailscaled socket {path}: {source} (is tailscaled running?)")]
    Connect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("localapi http error: {0}")]
    Http(#[from] hyper::Error),
    #[error("localapi request build error: {0}")]
    Request(#[from] hyper::http::Error),
    /// Non-2xx response; body is tailscaled's error text.
    #[error("tailscaled returned {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("cannot decode tailscaled response: {0}")]
    Decode(#[from] serde_json::Error),
}

pub struct LocalApi {
    socket: PathBuf,
}

impl LocalApi {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    /// Sends one request and returns the response body, requiring a 2xx
    /// status.
    async fn request(
        &self,
        method: Method,
        path_and_query: &str,
        body: Bytes,
    ) -> Result<Bytes, Error> {
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|source| Error::Connect {
                path: self.socket.clone(),
                source,
            })?;
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
        // Drive the connection; it ends when the request completes.
        let driver = tokio::spawn(conn);

        let req = Request::builder()
            .method(method)
            .uri(path_and_query)
            .header(header::HOST, HOST)
            .body(Full::new(body))?;
        let resp = sender.send_request(req).await?;
        let status = resp.status();
        let body = resp.into_body().collect().await?.to_bytes();
        drop(sender);
        driver.abort();

        if !status.is_success() {
            return Err(Error::Api {
                status,
                body: String::from_utf8_lossy(&body).trim().to_string(),
            });
        }
        Ok(body)
    }

    /// `GET /localapi/v0/status` as raw JSON bytes (for `status --json`).
    pub async fn status_raw(&self) -> Result<Bytes, Error> {
        self.request(Method::GET, "/localapi/v0/status", Bytes::new())
            .await
    }

    pub async fn status(&self) -> Result<ts_types::Status, Error> {
        Ok(serde_json::from_slice(&self.status_raw().await?)?)
    }

    /// `PATCH /localapi/v0/prefs` with a masked edit; returns the new prefs.
    pub async fn edit_prefs(
        &self,
        masked: &ts_types::MaskedPrefs,
    ) -> Result<ts_types::Prefs, Error> {
        let body = serde_json::to_vec(masked)?;
        let body = self
            .request(Method::PATCH, "/localapi/v0/prefs", Bytes::from(body))
            .await?;
        Ok(serde_json::from_slice(&body)?)
    }

    /// `POST /localapi/v0/ping?ip=…&type=disco`. Blocks until pong or
    /// tailscaled's own timeout.
    pub async fn ping(&self, ip: std::net::IpAddr) -> Result<ts_types::PingResult, Error> {
        let path = format!("/localapi/v0/ping?ip={ip}&type=disco");
        let body = self.request(Method::POST, &path, Bytes::new()).await?;
        Ok(serde_json::from_slice(&body)?)
    }
}
