//! The controlhttp upgrade dance: fetch the server's Noise key over plain
//! HTTP, then `POST /ts2021` with an `Upgrade` to switch the raw TCP stream
//! into the Noise transport. Mirrors Go `control/controlhttp/client.go`
//! (see PROTOCOL.md).
//!
//! Phase-2 scope is plain HTTP against Headscale (`http://host:port`). TLS
//! (:443) and the 80/443 race are deferred to the hosted-control-plane
//! milestone.
//!
//! HTTP/1.1 framing itself (the head parse/serialize, and reclaiming any
//! Noise bytes bundled with the upgrade response) is `rusty_http`'s job, not
//! hand-rolled here anymore -- see `DESIGN.md`'s dependency table.

use tokio::net::TcpStream;
use ts_key::MachinePrivate;

use rusty_http::head::RequestHead;
use rusty_http::tokio_native::{AsyncTransport, Replay};
use rusty_http::{HeaderMap, Method, Version};

use crate::controlbase::{self, Conn, ConnectError, client_initiation};

const UPGRADE_PATH: &str = "/ts2021";
const UPGRADE_HEADER_VALUE: &str = "tailscale-control-protocol";
const HANDSHAKE_HEADER: &str = "X-Tailscale-Handshake";
/// Cap on the HTTP/1.1 head (status line + headers) we'll buffer before
/// giving up -- generous for a control server's own responses, small enough
/// to bound a hung/malicious peer.
const MAX_HEAD_LEN: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ControlHttpError {
    #[error("control URL must be http://host[:port] (got {0:?})")]
    BadUrl(String),
    #[error("I/O error talking to control server: {0}")]
    Io(#[from] std::io::Error),
    #[error("control server returned HTTP {0} for {1}")]
    HttpStatus(u16, String),
    #[error("could not parse control server response: {0}")]
    Parse(String),
    #[error("server did not switch protocols (got status {0})")]
    NoUpgrade(u16),
    #[error(transparent)]
    Handshake(#[from] ConnectError),
    #[error("HTTP transport error: {0}")]
    Http(#[from] rusty_http::TransportError),
}

/// A parsed `http://host:port` control URL.
#[derive(Debug, Clone)]
pub struct ControlUrl {
    pub host: String,
    pub port: u16,
    /// Value for the HTTP `Host` header (host plus non-default port).
    pub authority: String,
}

impl ControlUrl {
    /// Parses `http://host[:port]`. HTTPS is not yet supported.
    pub fn parse(url: &str) -> Result<Self, ControlHttpError> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| ControlHttpError::BadUrl(url.to_string()))?;
        // Strip any path/query; we only need the authority.
        let authority_str = rest.split(['/', '?']).next().unwrap_or(rest);
        if authority_str.is_empty() {
            return Err(ControlHttpError::BadUrl(url.to_string()));
        }
        let (host, port) = match authority_str.rsplit_once(':') {
            Some((h, p)) => {
                let port = p
                    .parse()
                    .map_err(|_| ControlHttpError::BadUrl(url.to_string()))?;
                (h.to_string(), port)
            }
            None => (authority_str.to_string(), 80),
        };
        if host.is_empty() {
            return Err(ControlHttpError::BadUrl(url.to_string()));
        }
        Ok(Self {
            authority: authority_str.to_string(),
            host,
            port,
        })
    }

    fn socket_addr(&self) -> (String, u16) {
        (self.host.clone(), self.port)
    }
}

/// Fetches the control server's Noise static public key via
/// `GET /key?v=<version>` (plain HTTP, outside the Noise channel).
pub async fn fetch_control_key(
    url: &ControlUrl,
    protocol_version: u16,
) -> Result<ts_types::MachinePublic, ControlHttpError> {
    let stream = TcpStream::connect(url.socket_addr()).await?;
    let mut transport = AsyncTransport::new(stream);

    let mut headers = HeaderMap::new();
    headers
        .insert("Host", &url.authority)
        .map_err(|e| ControlHttpError::Parse(e.to_string()))?;
    headers
        .insert("Connection", "close")
        .map_err(|e| ControlHttpError::Parse(e.to_string()))?;
    transport
        .write_request_head(&RequestHead {
            method: Method::Get,
            target: format!("/key?v={protocol_version}"),
            version: Version::Http11,
            headers,
        })
        .await?;

    let head = transport.read_response_head(MAX_HEAD_LEN).await?;
    if head.status.as_u16() != 200 {
        return Err(ControlHttpError::HttpStatus(
            head.status.as_u16(),
            "/key".into(),
        ));
    }
    let framing = rusty_http::body::response_framing(&head.headers, &Method::Get, head.status)
        .map_err(|e| ControlHttpError::Parse(e.to_string()))?;
    let body = transport.read_body(framing).await?;

    #[derive(serde::Deserialize)]
    struct KeyResponse {
        #[serde(rename = "publicKey")]
        public_key: ts_types::MachinePublic,
    }
    let parsed: KeyResponse =
        serde_json::from_slice(&body).map_err(|e| ControlHttpError::Parse(e.to_string()))?;
    Ok(parsed.public_key)
}

/// Dials the control server and upgrades the connection to the Noise
/// transport. Returns the secured [`Conn`] ready for HTTP/2.
pub async fn dial(
    url: &ControlUrl,
    machine_key: &MachinePrivate,
    control_key: &[u8; 32],
    protocol_version: u16,
) -> Result<Conn<Replay<TcpStream>>, ControlHttpError> {
    let (init, handshake) = client_initiation(machine_key, control_key, protocol_version);

    let stream = TcpStream::connect(url.socket_addr()).await?;
    let mut transport = AsyncTransport::new(stream);

    let mut headers = HeaderMap::new();
    headers
        .insert("Host", &url.authority)
        .map_err(|e| ControlHttpError::Parse(e.to_string()))?;
    headers
        .insert("Upgrade", UPGRADE_HEADER_VALUE)
        .map_err(|e| ControlHttpError::Parse(e.to_string()))?;
    headers
        .insert("Connection", "upgrade")
        .map_err(|e| ControlHttpError::Parse(e.to_string()))?;
    headers
        .insert(HANDSHAKE_HEADER, &rusty_base64::encode_standard(&init))
        .map_err(|e| ControlHttpError::Parse(e.to_string()))?;
    headers
        .insert("Content-Length", "0")
        .map_err(|e| ControlHttpError::Parse(e.to_string()))?;
    transport
        .write_request_head(&RequestHead {
            method: Method::Post,
            target: UPGRADE_PATH.to_string(),
            version: Version::Http11,
            headers,
        })
        .await?;

    // Read only the HTTP response head; `rusty_http` consumes exactly the
    // head, so any bytes bundled with it in the same read belong to the
    // Noise stream, not this parse -- reclaim them via `into_parts` below
    // rather than let a plain `into_inner` silently drop them.
    let head = transport.read_response_head(MAX_HEAD_LEN).await?;
    if head.status.as_u16() != 101 {
        return Err(ControlHttpError::NoUpgrade(head.status.as_u16()));
    }
    let (stream, leftover) = transport.into_parts();
    let io = Replay::new(leftover, stream);

    // `io` now carries the Noise response + records, replaying any of it
    // that arrived bundled with the upgrade response.
    Ok(controlbase::connect_deferred(io, handshake).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parsing() {
        let u = ControlUrl::parse("http://127.0.0.1:8080").unwrap();
        assert_eq!(u.host, "127.0.0.1");
        assert_eq!(u.port, 8080);
        assert_eq!(u.authority, "127.0.0.1:8080");

        let u = ControlUrl::parse("http://headscale.example.com").unwrap();
        assert_eq!(u.host, "headscale.example.com");
        assert_eq!(u.port, 80);

        let u = ControlUrl::parse("http://host:8080/path?x=1").unwrap();
        assert_eq!(u.authority, "host:8080");

        assert!(ControlUrl::parse("https://x").is_err());
        assert!(ControlUrl::parse("ftp://x").is_err());
        assert!(ControlUrl::parse("http://").is_err());
    }

    // HTTP/1.1 head parsing itself (status-line + header parsing, and the
    // byte-exact-consumption guarantee the Noise handoff depends on) is
    // `rusty_http`'s job now and is covered by its own test suite -- see
    // `rusty_http::head`'s tests, not duplicated here.
}
