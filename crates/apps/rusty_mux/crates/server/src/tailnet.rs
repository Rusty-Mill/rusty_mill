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
use rusty_mux_core::TailscaleState;
use rusty_tokio::io::UnixStream;
use ts_types::Status;
use ts_types::tailcfg::WhoIsResponse;

/// Where `tailscaled` listens on Linux.
pub const DEFAULT_SOCKET: &str = "/var/run/tailscale/tailscaled.sock";

const HOST: &str = "local-tailscaled.sock";
/// The most response head we buffer; the body is a short JSON document.
const MAX_HEAD_LEN: usize = 64 * 1024;
/// How long one lookup, connect to last byte, may take.
pub const DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);
/// The login `tailscaled` gives every tagged node: shared, so never an identity.
const TAGGED_LOGIN: &str = "tagged-devices";

/// Whether `ip` is a tailnet address: `100.64.0.0/10` or `fd7a:115c:a1e0::/48`.
/// A listener binds only to one, so a peer's address is one `tailscaled` can
/// name; loopback and every other address mean nothing to it.
pub fn is_tailnet_addr(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => v4.octets()[0] == 100 && v4.octets()[1] & 0xc0 == 64,
        IpAddr::V6(v6) => v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

/// A client of `tailscaled`'s LocalAPI.
#[derive(Debug, Clone)]
pub struct Whois {
    socket: PathBuf,
}

impl Whois {
    /// A client of the LocalAPI socket at `socket`, [`DEFAULT_SOCKET`] on Linux.
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    /// The principal behind `peer`, an address on the tailnet. A stalled
    /// `tailscaled` fails the call after [`DEADLINE`].
    ///
    /// A peer at one of this node's own addresses is refused: any local
    /// process can dial the node's tailnet address, and `tailscaled` would
    /// name it as the node's owner.
    pub async fn principal(&self, peer: SocketAddr) -> io::Result<Principal> {
        let target = format!("/localapi/v0/whois?addr={}", encode_addr(peer));
        let (body, own) = rusty_tokio::time::timeout(DEADLINE, async {
            let body = self.get(target, "whois").await?;
            Ok::<_, io::Error>((body, self.own_addrs().await?))
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "tailscaled whois timed out"))??;
        if own.contains(&peer.ip()) {
            return Err(invalid("the peer is this node itself: a local process"));
        }
        let who: WhoIsResponse =
            serde_json::from_slice(&body).map_err(|e| invalid(format!("whois reply: {e}")))?;
        principal_of(&who).map_err(invalid)
    }

    /// Where this node stands: `tailscaled`'s backend state, and the node's
    /// own name when it has one. Fails after [`DEADLINE`] like a lookup.
    pub async fn status(&self) -> io::Result<(TailscaleState, Option<String>)> {
        let target = "/localapi/v0/status?peers=false".to_owned();
        let body = rusty_tokio::time::timeout(DEADLINE, self.get(target, "status"))
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "tailscaled status timed out")
            })??;
        let status: Status =
            serde_json::from_slice(&body).map_err(|e| invalid(format!("status reply: {e}")))?;
        Ok(state_of(&status))
    }

    /// This node's own tailnet addresses, from `tailscaled`'s status.
    async fn own_addrs(&self) -> io::Result<Vec<std::net::IpAddr>> {
        let body = self
            .get("/localapi/v0/status?peers=false".to_owned(), "status")
            .await?;
        let status: ts_types::Status =
            serde_json::from_slice(&body).map_err(|e| invalid(format!("status reply: {e}")))?;
        Ok(status.tailscale_ips)
    }

    async fn get(&self, target: String, what: &str) -> io::Result<Vec<u8>> {
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
            target,
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
                "tailscaled {what}: status {}: {}",
                reply.status,
                String::from_utf8_lossy(&body).trim()
            )));
        }
        Ok(body)
    }
}

/// The principal for a decoded answer: a tagged node is its node name
/// (`ts-node:NAME`), anything else its user's login (`ts:LOGIN`). A peer
/// `tailscaled` does not know, or one with no name or login of its own, is
/// refused. `user` stays empty: it names a local account in login routing,
/// and a tailnet name must never be taken for one; route by `id`.
pub fn principal_of(who: &WhoIsResponse) -> Result<Principal, String> {
    let node = who
        .node
        .as_ref()
        .ok_or("tailscaled does not know the peer")?;
    let id = if node.tags.is_empty() {
        let login = who
            .user_profile
            .as_ref()
            .map(|u| u.login_name.as_str())
            .filter(|l| !l.is_empty() && *l != TAGGED_LOGIN)
            .ok_or("the peer's node has no user")?;
        format!("ts:{login}")
    } else {
        let name = node.name.trim_end_matches('.');
        if name.is_empty() {
            return Err("the peer's tagged node has no name".into());
        }
        format!("ts-node:{name}")
    };
    Principal::new(id, "").map_err(|e| e.to_string())
}

/// The state and node name a status reply says. The node's name is its
/// DNS name without the trailing dot, if it has one.
fn state_of(status: &Status) -> (TailscaleState, Option<String>) {
    let node = status
        .self_
        .as_ref()
        .map(|me| me.dns_name.trim_end_matches('.'))
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    (TailscaleState::from_backend(&status.backend_state), node)
}

/// `addr` as a query value: `%` (an IPv6 zone) and an IPv6 peer's brackets
/// are escaped.
fn encode_addr(peer: SocketAddr) -> String {
    peer.to_string()
        .replace('%', "%25")
        .replace('[', "%5B")
        .replace(']', "%5D")
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn http(err: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("tailscaled LocalAPI: {err}"))
}

#[cfg(all(test, unix))]
mod tests;
