//! Minimal LocalAPI client: HTTP/1.1 over the tailscaled Unix socket.
//!
//! Mirrors the Go client (`client/tailscale/localclient.go`): requests target
//! `http://local-tailscaled.sock/localapi/v0/...` with the `Host` header set
//! to `local-tailscaled.sock`; on Linux authentication is by socket peer
//! credentials. One connection per request — a CLI has no use for a pool.
//!
//! HTTP/1.1 framing is `rusty_http`'s job, not `hyper`'s, anymore -- see
//! `DESIGN.md`'s dependency table.

#![allow(dead_code, unused_imports)]

use std::path::PathBuf;

use rusty_http::head::RequestHead;
use rusty_http::tokio_native::AsyncTransport;
use rusty_http::{HeaderMap, Method, StatusCode, Version};
#[cfg(unix)]
use tokio::net::UnixStream;

/// Default tailscaled socket path on Linux.
pub const DEFAULT_SOCKET: &str = "/var/run/tailscale/tailscaled.sock";

const HOST: &str = "local-tailscaled.sock";
/// Cap on the HTTP/1.1 response head we'll buffer before giving up.
const MAX_HEAD_LEN: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot connect to tailscaled socket {path}: {source} (is tailscaled running?)")]
    Connect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("LocalAPI HTTP transport error: {0}")]
    Http(#[from] rusty_http::TransportError),
    #[error("LocalAPI request construction error: {0}")]
    Request(#[from] rusty_http::Error),
    #[error("LocalAPI HTTP status {0}")]
    Status(StatusCode),
    #[error("LocalAPI JSON decode error: {0}")]
    Decode(#[from] serde_json::Error),
}

pub struct LocalApi {
    socket: PathBuf,
}

impl LocalApi {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    #[cfg(not(unix))]
    async fn request(
        &self,
        _method: Method,
        _path_and_query: &str,
        _body: Vec<u8>,
    ) -> Result<Vec<u8>, Error> {
        Err(Error::Connect {
            path: self.socket.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Unix domain sockets are not supported on this platform",
            ),
        })
    }

    /// Sends one request and returns the response body, requiring a 2xx
    /// status.
    #[cfg(unix)]
    async fn request(
        &self,
        method: Method,
        path_and_query: &str,
        body: Vec<u8>,
    ) -> Result<Vec<u8>, Error> {
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|source| Error::Connect {
                path: self.socket.clone(),
                source,
            })?;
        let mut transport = AsyncTransport::new(stream);

        let mut headers = HeaderMap::new();
        headers.insert("Host", HOST)?;
        headers.insert("Content-Length", &body.len().to_string())?;
        transport
            .write_request_head(&RequestHead {
                method: method.clone(),
                target: path_and_query.to_string(),
                version: Version::Http11,
                headers,
            })
            .await?;
        transport.write_body(&body).await?;

        let head = transport.read_response_head(MAX_HEAD_LEN).await?;
        let framing = rusty_http::body::response_framing(&head.headers, &method, head.status)?;
        let body = transport.read_body(framing).await?;

        if !head.status.is_success() {
            return Err(Error::Api {
                status: head.status,
                body: String::from_utf8_lossy(&body).trim().to_string(),
            });
        }
        Ok(body)
    }

    /// `GET /localapi/v0/status` as raw JSON bytes (for `status --json`).
    pub async fn status_raw(&self) -> Result<Vec<u8>, Error> {
        self.request(Method::Get, "/localapi/v0/status", Vec::new())
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
            .request(Method::Patch, "/localapi/v0/prefs", body)
            .await?;
        Ok(serde_json::from_slice(&body)?)
    }

    /// `POST /localapi/v0/ping?ip=…&type=disco`. Blocks until pong or
    /// tailscaled's own timeout.
    pub async fn ping(&self, ip: std::net::IpAddr) -> Result<ts_types::PingResult, Error> {
        let path = format!("/localapi/v0/ping?ip={ip}&type=disco");
        let body = self.request(Method::Post, &path, Vec::new()).await?;
        Ok(serde_json::from_slice(&body)?)
    }
}
