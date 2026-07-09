//! The controlhttp upgrade dance: fetch the server's Noise key over plain
//! HTTP, then `POST /ts2021` with an `Upgrade` to switch the raw TCP stream
//! into the Noise transport. Mirrors Go `control/controlhttp/client.go`
//! (see PROTOCOL.md).
//!
//! Phase-2 scope is plain HTTP against Headscale (`http://host:port`). TLS
//! (:443) and the 80/443 race are deferred to the hosted-control-plane
//! milestone.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use ts_key::MachinePrivate;

use crate::base64;
use crate::controlbase::{self, Conn, ConnectError, client_initiation};

const UPGRADE_PATH: &str = "/ts2021";
const UPGRADE_HEADER_VALUE: &str = "tailscale-control-protocol";
const HANDSHAKE_HEADER: &str = "X-Tailscale-Handshake";

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
    let mut stream = TcpStream::connect(url.socket_addr()).await?;
    let req = format!(
        "GET /key?v={protocol_version} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        url.authority
    );
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    let (status, _headers, body) = parse_http_response(&raw)?;
    if status != 200 {
        return Err(ControlHttpError::HttpStatus(status, "/key".into()));
    }

    #[derive(serde::Deserialize)]
    struct KeyResponse {
        #[serde(rename = "publicKey")]
        public_key: ts_types::MachinePublic,
    }
    let parsed: KeyResponse =
        serde_json::from_slice(body).map_err(|e| ControlHttpError::Parse(e.to_string()))?;
    Ok(parsed.public_key)
}

/// Dials the control server and upgrades the connection to the Noise
/// transport. Returns the secured [`Conn`] ready for HTTP/2.
pub async fn dial(
    url: &ControlUrl,
    machine_key: &MachinePrivate,
    control_key: &[u8; 32],
    protocol_version: u16,
) -> Result<Conn<TcpStream>, ControlHttpError> {
    let (init, handshake) = client_initiation(machine_key, control_key, protocol_version);

    let mut stream = TcpStream::connect(url.socket_addr()).await?;
    let req = format!(
        "POST {UPGRADE_PATH} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Upgrade: {UPGRADE_HEADER_VALUE}\r\n\
         Connection: upgrade\r\n\
         {HANDSHAKE_HEADER}: {handshake_b64}\r\n\
         Content-Length: 0\r\n\r\n",
        host = url.authority,
        handshake_b64 = base64::encode(&init),
    );
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;

    // Read only the HTTP response head (up to and including the blank line);
    // anything after belongs to the Noise stream, so we must not over-read.
    // Reading a byte at a time guarantees we stop exactly at the CRLFCRLF.
    let status = read_http_head(&mut stream).await?;
    if status != 101 {
        return Err(ControlHttpError::NoUpgrade(status));
    }

    // The raw TCP stream now carries the Noise response + records.
    Ok(controlbase::connect_deferred(stream, handshake).await?)
}

/// Reads an HTTP/1.1 response head byte-by-byte until the CRLFCRLF
/// terminator (so we don't consume Noise bytes), returning the status code.
async fn read_http_head(stream: &mut TcpStream) -> Result<u16, ControlHttpError> {
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            return Err(ControlHttpError::Parse(
                "connection closed before HTTP head".into(),
            ));
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
        if head.len() > 64 * 1024 {
            return Err(ControlHttpError::Parse("HTTP head too large".into()));
        }
    }
    parse_status_line(&head)
}

fn parse_status_line(head: &[u8]) -> Result<u16, ControlHttpError> {
    let line_end = head
        .windows(2)
        .position(|w| w == b"\r\n")
        .ok_or_else(|| ControlHttpError::Parse("no status line".into()))?;
    let line = std::str::from_utf8(&head[..line_end])
        .map_err(|_| ControlHttpError::Parse("non-UTF8 status line".into()))?;
    // "HTTP/1.1 101 Switching Protocols"
    let mut parts = line.split(' ');
    let _version = parts.next();
    let code = parts
        .next()
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| ControlHttpError::Parse(format!("bad status line {line:?}")))?;
    Ok(code)
}

/// Splits a complete HTTP response buffer into (status, header-bytes, body).
fn parse_http_response(raw: &[u8]) -> Result<(u16, &[u8], &[u8]), ControlHttpError> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| ControlHttpError::Parse("no header/body separator".into()))?;
    let status = parse_status_line(raw)?;
    let headers = &raw[..sep];
    let body = &raw[sep + 4..];
    Ok((status, headers, body))
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

    #[test]
    fn status_line_parsing() {
        assert_eq!(
            parse_status_line(b"HTTP/1.1 101 Switching Protocols\r\n").unwrap(),
            101
        );
        assert_eq!(parse_status_line(b"HTTP/1.1 200 OK\r\n").unwrap(), 200);
        assert!(parse_status_line(b"garbage\r\n").is_err());
    }

    #[test]
    fn http_response_split() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        let (status, _h, body) = parse_http_response(raw).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
    }
}
