//! Tailscale identity (C27): who a tailnet address belongs to.
//!
//! The server asks the local `tailscaled` (`GET /localapi/v0/whois`, over
//! its Unix socket, as the `tailscale` CLI does) and turns the answer into a
//! [`Principal`]. WireGuard has already authenticated the peer, so the
//! answer is as trustworthy as `tailscaled`; the socket's own permissions
//! decide who may ask. A listener (a later slice) admits the principal only
//! if the access table names it, as for any other principal.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

use rusty_http::async_tokio::AsyncTransport;
use rusty_http::head::RequestHead;
use rusty_http::{HeaderMap, Method, Version};
use rusty_mux_core::Principal;
use rusty_tokio::io::UnixStream;
use ts_types::tailcfg::WhoIsResponse;

/// Where `tailscaled` listens on Linux.
pub const DEFAULT_SOCKET: &str = "/var/run/tailscale/tailscaled.sock";

const HOST: &str = "local-tailscaled.sock";
/// The most response head we buffer; the body is a short JSON document.
const MAX_HEAD_LEN: usize = 64 * 1024;

/// A client of `tailscaled`'s LocalAPI.
#[derive(Debug, Clone)]
pub struct Whois {
    socket: PathBuf,
}

impl Whois {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    /// The principal behind `peer`, an address on the tailnet.
    pub async fn principal(&self, peer: SocketAddr) -> io::Result<Principal> {
        let who: WhoIsResponse = serde_json::from_slice(&self.get(peer).await?)
            .map_err(|e| invalid(format!("whois reply: {e}")))?;
        principal_of(&who).map_err(invalid)
    }

    async fn get(&self, peer: SocketAddr) -> io::Result<Vec<u8>> {
        let stream = UnixStream::connect(&self.socket).await.map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("tailscaled socket {}: {e}", self.socket.display()),
            )
        })?;
        let mut transport = AsyncTransport::new(stream);
        let mut headers = HeaderMap::new();
        headers.insert("Host", HOST).map_err(http)?;
        headers.insert("Content-Length", "0").map_err(http)?;
        let head = RequestHead {
            method: Method::Get,
            target: format!("/localapi/v0/whois?addr={}", encode_addr(peer)),
            version: Version::Http11,
            headers,
        };
        transport.write_request_head(&head).await.map_err(http)?;
        let reply = transport
            .read_response_head(MAX_HEAD_LEN)
            .await
            .map_err(http)?;
        let framing =
            rusty_http::body::response_framing(&reply.headers, &Method::Get, reply.status)
                .map_err(http)?;
        let body = transport.read_body(framing).await.map_err(http)?;
        if !reply.status.is_success() {
            return Err(io::Error::other(format!(
                "tailscaled whois {peer}: status {}: {}",
                reply.status,
                String::from_utf8_lossy(&body).trim()
            )));
        }
        Ok(body)
    }
}

/// `principal` for a decoded answer: a tagged node is its node name
/// (`ts-node:NAME`), anything else its user's login (`ts:LOGIN`). A peer
/// `tailscaled` does not know is refused.
pub fn principal_of(who: &WhoIsResponse) -> Result<Principal, String> {
    let node = who
        .node
        .as_ref()
        .ok_or("tailscaled does not know the peer")?;
    let (id, user) = if node.tags.is_empty() {
        let login = who
            .user_profile
            .as_ref()
            .map(|u| u.login_name.as_str())
            .filter(|l| !l.is_empty())
            .ok_or("the peer's node has no user")?;
        (format!("ts:{login}"), login.to_owned())
    } else {
        let name = node.name.trim_end_matches('.');
        (format!("ts-node:{name}"), name.to_owned())
    };
    Principal::new(id, user).map_err(|e| e.to_string())
}

/// `addr` as a query value: an IPv6 peer's brackets are escaped.
fn encode_addr(peer: SocketAddr) -> String {
    peer.to_string().replace('[', "%5B").replace(']', "%5D")
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn http(err: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("tailscaled LocalAPI: {err}"))
}

#[cfg(all(test, unix))]
mod tests;
