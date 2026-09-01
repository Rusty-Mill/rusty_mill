//! Async DERP client: HTTP upgrade, NaCl-box handshake, and a split
//! send/receive path over the relay. Mirrors Go `derp/derp_client.go` +
//! `derphttp` (see PROTOCOL.md).
//!
//! After [`DerpClient::connect`] the client runs two background tasks — a
//! writer fed by an mpsc queue and a reader that dispatches control frames
//! (ping→pong, keepalive, peer-gone) itself and forwards relayed packets to
//! an inbound channel.
//!
//! HTTP/1.1 framing for the upgrade itself (head parse/serialize, and
//! reclaiming any DERP frame bytes bundled with the upgrade response) is
//! `rusty_http`'s job, not hand-rolled here anymore -- see `DESIGN.md`'s
//! dependency table.

use crypto_box::{
    PublicKey, SalsaBox, SecretKey,
    aead::{Aead, AeadCore, OsRng},
};
use rusty_http::head::RequestHead;
use rusty_http::tokio_native::{AsyncTransport, Replay};
use rusty_http::{HeaderMap, Method, Version};
use tokio::io::AsyncRead;
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use ts_key::NodePrivate;
use ts_types::NodePublic;

use crate::frame::{self, FrameType, MAGIC, MAX_FRAME_SIZE, MAX_PACKET_SIZE};

const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
/// Bound on the ClientInfo/ServerInfo JSON we'll process.
const MAX_INFO_LEN: usize = 4 << 10;
/// Cap on the HTTP/1.1 upgrade response head we'll buffer before giving up.
const MAX_HEAD_LEN: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DerpError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame error: {0}")]
    Frame(#[from] frame::FrameError),
    #[error("bad DERP URL {0:?} (want http://host:port)")]
    BadUrl(String),
    #[error("HTTP upgrade failed: {0}")]
    Upgrade(String),
    #[error("HTTP transport error: {0}")]
    Http(#[from] rusty_http::TransportError),
    #[error("invalid server greeting (expected ServerKey frame with magic)")]
    BadGreeting,
    #[error("server info naclbox failed to open")]
    BadServerInfo,
    #[error("packet exceeds max size ({0} bytes)")]
    PacketTooBig(usize),
    #[error("DERP connection closed")]
    Closed,
}

/// A relayed packet: the peer node key and opaque payload.
#[derive(Debug, Clone)]
pub struct RelayedPacket {
    /// For inbound: source node. For outbound: destination node.
    pub peer: NodePublic,
    pub payload: Vec<u8>,
}

/// A connected DERP client.
pub struct DerpClient {
    /// The relay server's node public key (from the greeting).
    server_key: NodePublic,
    /// Outbound queue → writer task.
    outbound: mpsc::Sender<Outbound>,
    /// Inbound relayed packets ← reader task.
    inbound: mpsc::Receiver<RelayedPacket>,
    _reader: TaskGuard,
    _writer: TaskGuard,
}

/// A cloneable send handle for a [`DerpClient`], usable while the client is
/// borrowed for [`DerpClient::recv`].
#[derive(Clone)]
pub struct DerpSender {
    outbound: mpsc::Sender<Outbound>,
}

impl DerpSender {
    /// Queues a packet for relay to `dst`.
    pub async fn send(&self, dst: NodePublic, payload: Vec<u8>) -> Result<(), DerpError> {
        if payload.len() > MAX_PACKET_SIZE {
            return Err(DerpError::PacketTooBig(payload.len()));
        }
        self.outbound
            .send(Outbound::Send { dst, payload })
            .await
            .map_err(|_| DerpError::Closed)
    }
}

/// Messages to the writer task.
enum Outbound {
    Send { dst: NodePublic, payload: Vec<u8> },
    Pong([u8; 8]),
}

impl DerpClient {
    /// Connects to a plain-HTTP DERP relay (`http://host:port`), performs
    /// the upgrade and NaCl-box handshake using `node_key` as this client's
    /// identity.
    pub async fn connect(url: &str, node_key: &NodePrivate) -> Result<Self, DerpError> {
        let (host_port, host_header) = parse_http_url(url)?;
        let stream = TcpStream::connect(&host_port).await?;
        stream.set_nodelay(true).ok();
        Self::handshake_over(stream, &host_header, node_key).await
    }

    /// The relay server's node public key.
    pub fn server_key(&self) -> NodePublic {
        self.server_key
    }

    /// A cloneable send handle, usable concurrently with [`Self::recv`].
    pub fn sender(&self) -> DerpSender {
        DerpSender {
            outbound: self.outbound.clone(),
        }
    }

    /// Queues a packet for relay to `dst`. Non-blocking against the network;
    /// applies backpressure only if the writer queue is full.
    pub async fn send(&self, dst: NodePublic, payload: Vec<u8>) -> Result<(), DerpError> {
        if payload.len() > MAX_PACKET_SIZE {
            return Err(DerpError::PacketTooBig(payload.len()));
        }
        self.outbound
            .send(Outbound::Send { dst, payload })
            .await
            .map_err(|_| DerpError::Closed)
    }

    /// Awaits the next relayed packet from a peer, or `None` when the
    /// connection has closed.
    pub async fn recv(&mut self) -> Option<RelayedPacket> {
        self.inbound.recv().await
    }

    /// Performs the upgrade + handshake over an already-connected stream.
    /// Split out so tests can drive it over an in-memory duplex.
    async fn handshake_over(
        stream: TcpStream,
        host_header: &str,
        node_key: &NodePrivate,
    ) -> Result<Self, DerpError> {
        let (mut read_half, mut write_half) = http_upgrade(stream, host_header).await?;

        // 1. Read ServerKey: 8B magic + 32B server node public key.
        let (t, payload) = frame::read_frame(&mut read_half, 1 << 10).await?;
        if t != FrameType::ServerKey || payload.len() < MAGIC.len() + KEY_LEN {
            return Err(DerpError::BadGreeting);
        }
        if &payload[..MAGIC.len()] != MAGIC {
            return Err(DerpError::BadGreeting);
        }
        let mut server_bytes = [0u8; KEY_LEN];
        server_bytes.copy_from_slice(&payload[MAGIC.len()..MAGIC.len() + KEY_LEN]);
        let server_key = NodePublic(server_bytes);

        // 2. Send ClientInfo: 32B our node pub + 24B nonce + naclbox(json).
        let salsa = SalsaBox::new(
            &PublicKey::from(server_bytes),
            &SecretKey::from(node_key.to_bytes()),
        );
        let info = br#"{"version":2,"CanAckPings":true}"#;
        let nonce = SalsaBox::generate_nonce(&mut OsRng);
        let boxed = salsa
            .encrypt(&nonce, &info[..])
            .map_err(|_| DerpError::BadServerInfo)?;
        let mut client_info = Vec::with_capacity(KEY_LEN + NONCE_LEN + boxed.len());
        client_info.extend_from_slice(&node_key.public().0);
        client_info.extend_from_slice(&nonce[..]);
        client_info.extend_from_slice(&boxed);
        frame::write_frame(&mut write_half, FrameType::ClientInfo, &client_info).await?;

        // 3. Read ServerInfo (naclbox), validate it opens with our key.
        let (t, payload) =
            frame::read_frame(&mut read_half, (NONCE_LEN + MAX_INFO_LEN) as u32).await?;
        if t == FrameType::ServerInfo {
            if payload.len() < NONCE_LEN {
                return Err(DerpError::BadServerInfo);
            }
            let (nonce_bytes, ct) = payload.split_at(NONCE_LEN);
            salsa
                .decrypt(nonce_bytes.into(), ct)
                .map_err(|_| DerpError::BadServerInfo)?;
        }
        // Some servers may send other frames first; we don't require
        // ServerInfo to proceed, but a present one must authenticate.

        // Spawn reader + writer tasks.
        let (outbound_tx, outbound_rx) = mpsc::channel::<Outbound>(256);
        let (inbound_tx, inbound_rx) = mpsc::channel::<RelayedPacket>(256);

        let pong_tx = outbound_tx.clone();
        let reader = tokio::spawn(reader_loop(read_half, inbound_tx, pong_tx));
        let writer = tokio::spawn(writer_loop(write_half, outbound_rx));

        Ok(DerpClient {
            server_key,
            outbound: outbound_tx,
            inbound: inbound_rx,
            _reader: TaskGuard(reader),
            _writer: TaskGuard(writer),
        })
    }
}

/// Reader task: dispatches control frames, forwards relayed packets inbound.
///
/// Generic over the read half's type (rather than the concrete
/// `OwnedReadHalf`) because [`http_upgrade`] may hand back a
/// [`Replay`]-wrapped one when the server's ServerKey greeting arrived
/// bundled with the HTTP upgrade response.
async fn reader_loop<R: AsyncRead + Unpin + Send + 'static>(
    mut read_half: R,
    inbound: mpsc::Sender<RelayedPacket>,
    pong: mpsc::Sender<Outbound>,
) {
    loop {
        let (t, payload) = match frame::read_frame(&mut read_half, MAX_FRAME_SIZE).await {
            Ok(f) => f,
            Err(e) => {
                tracing::debug!("DERP read loop ended: {e}");
                return;
            }
        };
        match t {
            FrameType::RecvPacket => {
                if payload.len() < KEY_LEN {
                    tracing::debug!("short RecvPacket frame");
                    continue;
                }
                let mut src = [0u8; KEY_LEN];
                src.copy_from_slice(&payload[..KEY_LEN]);
                let packet = RelayedPacket {
                    peer: NodePublic(src),
                    payload: payload[KEY_LEN..].to_vec(),
                };
                if inbound.send(packet).await.is_err() {
                    return; // consumer gone
                }
            }
            FrameType::Ping => {
                if payload.len() == 8 {
                    let mut echo = [0u8; 8];
                    echo.copy_from_slice(&payload);
                    let _ = pong.send(Outbound::Pong(echo)).await;
                }
            }
            FrameType::KeepAlive | FrameType::Pong => {}
            FrameType::PeerGone => tracing::debug!("DERP peer gone"),
            FrameType::PeerPresent => tracing::debug!("DERP peer present"),
            FrameType::Health => {
                if !payload.is_empty() {
                    tracing::warn!("DERP health: {}", String::from_utf8_lossy(&payload));
                }
            }
            FrameType::Restarting => tracing::info!("DERP server restarting"),
            other => tracing::trace!("ignoring DERP frame {other:?}"),
        }
    }
}

/// Writer task: drains the outbound queue to the relay.
async fn writer_loop(mut write_half: OwnedWriteHalf, mut rx: mpsc::Receiver<Outbound>) {
    while let Some(msg) = rx.recv().await {
        let result = match msg {
            Outbound::Send { dst, payload } => {
                let mut buf = Vec::with_capacity(KEY_LEN + payload.len());
                buf.extend_from_slice(&dst.0);
                buf.extend_from_slice(&payload);
                frame::write_frame(&mut write_half, FrameType::SendPacket, &buf).await
            }
            Outbound::Pong(echo) => {
                frame::write_frame(&mut write_half, FrameType::Pong, &echo).await
            }
        };
        if let Err(e) = result {
            tracing::debug!("DERP write loop ended: {e}");
            return;
        }
    }
}

/// Aborts a background task when the client is dropped.
struct TaskGuard(tokio::task::JoinHandle<()>);
impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Sends the `GET /derp` upgrade request and consumes the `101` response
/// head, then splits the stream: the server can push its ServerKey greeting
/// frame in the same read as the upgrade response, so any such bytes are
/// reclaimed via `into_parts` and replayed into the read half rather than
/// split off (and silently dropped) beforehand.
async fn http_upgrade(
    stream: TcpStream,
    host: &str,
) -> Result<(Replay<OwnedReadHalf>, OwnedWriteHalf), DerpError> {
    let mut transport = AsyncTransport::new(stream);

    let mut headers = HeaderMap::new();
    headers
        .insert("Host", host)
        .map_err(|e| DerpError::Upgrade(e.to_string()))?;
    headers
        .insert("Upgrade", "DERP")
        .map_err(|e| DerpError::Upgrade(e.to_string()))?;
    headers
        .insert("Connection", "Upgrade")
        .map_err(|e| DerpError::Upgrade(e.to_string()))?;
    transport
        .write_request_head(&RequestHead {
            method: Method::Get,
            target: "/derp".to_string(),
            version: Version::Http11,
            headers,
        })
        .await?;

    let head = transport.read_response_head(MAX_HEAD_LEN).await?;
    if head.status.as_u16() != 101 {
        return Err(DerpError::Upgrade(format!(
            "server did not switch protocols (got status {})",
            head.status.as_u16()
        )));
    }

    let (stream, leftover) = transport.into_parts();
    let (read_half, write_half) = stream.into_split();
    Ok((Replay::new(leftover, read_half), write_half))
}

/// Parses `http://host:port[/path]` into (`host:port`, host-header value).
fn parse_http_url(url: &str) -> Result<(String, String), DerpError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| DerpError::BadUrl(url.to_string()))?;
    let authority = rest.split(['/', '?']).next().unwrap_or(rest);
    if authority.is_empty() || !authority.contains(':') {
        return Err(DerpError::BadUrl(url.to_string()));
    }
    Ok((authority.to_string(), authority.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parsing() {
        assert_eq!(
            parse_http_url("http://127.0.0.1:8080/derp").unwrap().0,
            "127.0.0.1:8080"
        );
        assert!(parse_http_url("https://x:1").is_err());
        assert!(parse_http_url("http://nohost").is_err());
    }
}
