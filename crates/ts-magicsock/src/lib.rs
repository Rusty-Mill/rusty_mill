//! magicsock: the path multiplexer. One virtual transport per peer that hides
//! path selection from WireGuard — every WG datagram rides either a direct
//! UDP endpoint or a DERP relay, and traffic migrates to a direct path once
//! disco proves one works, falling back to DERP if it goes stale.
//!
//! This is the deliberately-de-risked version of Go's magicsock: a real UDP
//! socket + disco (ping/pong/call-me-maybe) + a typed per-peer path state
//! machine. STUN gives the server-reflexive endpoint for NAT traversal; local
//! interface endpoints cover flat networks. Endpoint discovery, hole
//! punching, and live DERP→direct migration all live here; WireGuard and the
//! control plane are untouched (ports-and-adapters).
//!
//! Protocol details: PROTOCOL.md.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand_core::{OsRng, RngCore};
use tokio::net::UdpSocket;
use ts_derp::DerpSender;
use ts_disco::{Message, TxId};
use ts_key::DiscoPrivate;
use ts_types::{DiscoPublic, NodePublic};

/// A verified direct path is considered stale if no pong arrives within this
/// window; traffic then falls back to DERP.
const PATH_STALE_AFTER: Duration = Duration::from_secs(15);
/// How often to re-ping to keep NAT mappings alive and detect path loss.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// How often to re-send call-me-maybe to a peer that has no direct path yet.
const CALL_ME_MAYBE_RESEND: Duration = Duration::from_secs(2);

/// Which path a peer's traffic currently takes (for status/metrics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// Relayed through DERP.
    Relay,
    /// A direct UDP path to this endpoint.
    Direct(SocketAddr),
}

/// The result of feeding an inbound UDP datagram to magicsock.
pub enum UdpInput {
    /// The datagram was a disco control message; magicsock handled it.
    HandledDisco,
    /// The datagram is a WireGuard packet from `peer`; hand it to WireGuard.
    WireGuard { peer: NodePublic, payload: Vec<u8> },
    /// From an unknown source and not disco; drop.
    Unknown,
}

struct PeerPaths {
    disco_key: DiscoPublic,
    /// Candidate endpoints learned via call-me-maybe / inbound pings.
    candidates: Vec<SocketAddr>,
    /// The current verified direct endpoint, if any, and when it last ponged.
    direct: Option<(SocketAddr, Instant)>,
    /// When we last sent this peer a call-me-maybe (for throttled re-sends
    /// until a direct path is up — the first one can race the peer's DERP
    /// registration and be dropped).
    last_call_me_maybe: Option<Instant>,
}

impl PeerPaths {
    fn path(&self) -> PathKind {
        match self.direct {
            Some((addr, last)) if last.elapsed() < PATH_STALE_AFTER => PathKind::Direct(addr),
            _ => PathKind::Relay,
        }
    }
}

/// The magic socket. Owned and driven by the engine's single task (so no
/// internal locking); the UDP socket is shared via `Arc` for the receive
/// path.
pub struct MagicSock {
    udp: Arc<UdpSocket>,
    port: u16,
    disco: DiscoPrivate,
    derp: DerpSender,
    peers: HashMap<NodePublic, PeerPaths>,
    disco_to_node: HashMap<DiscoPublic, NodePublic>,
    /// Verified/candidate endpoint → peer, for classifying inbound WG.
    endpoint_to_node: HashMap<SocketAddr, NodePublic>,
    /// In-flight disco pings: tx id → (peer, endpoint pinged).
    pending: HashMap<TxId, (NodePublic, SocketAddr)>,
    /// Our own candidate endpoints (local + reflexive) advertised to peers.
    local_endpoints: Vec<SocketAddr>,
}

impl MagicSock {
    /// Binds the UDP socket (IPv4, `0.0.0.0:0` unless `port` is set) and
    /// builds the mux.
    pub async fn new(port: u16, disco: DiscoPrivate, derp: DerpSender) -> std::io::Result<Self> {
        let udp = UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).await?;
        let bound = udp.local_addr()?.port();
        Ok(Self {
            udp: Arc::new(udp),
            port: bound,
            disco,
            derp,
            peers: HashMap::new(),
            disco_to_node: HashMap::new(),
            endpoint_to_node: HashMap::new(),
            pending: HashMap::new(),
            local_endpoints: Vec::new(),
        })
    }

    /// The shared UDP socket, for the engine's receive loop.
    pub fn udp(&self) -> Arc<UdpSocket> {
        self.udp.clone()
    }

    /// Learns our server-reflexive endpoint by sending a STUN binding request
    /// from the magicsock UDP socket (so the mapping matches). Call at startup,
    /// before the receive loop owns the socket. `None` if the server doesn't
    /// answer.
    pub async fn discover_reflexive(&self, stun_server: &str) -> Option<SocketAddr> {
        let stun_addr = tokio::net::lookup_host(stun_server).await.ok()?.next()?;
        let tx = ts_stun::TxId::random();
        self.udp
            .send_to(&ts_stun::binding_request(tx), stun_addr)
            .await
            .ok()?;
        let mut buf = [0u8; 512];
        let (n, _) = tokio::time::timeout(Duration::from_secs(3), self.udp.recv_from(&mut buf))
            .await
            .ok()?
            .ok()?;
        let reflexive = ts_stun::parse_response(&buf[..n], tx).ok()?;
        tracing::info!(%reflexive, "magicsock: learned reflexive endpoint via STUN");
        Some(reflexive)
    }

    /// The port the UDP socket is bound to.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Registers a peer's disco key (from the netmap).
    pub fn add_peer(&mut self, node: NodePublic, disco_key: DiscoPublic) {
        self.disco_to_node.insert(disco_key, node);
        self.peers.entry(node).or_insert(PeerPaths {
            disco_key,
            candidates: Vec::new(),
            direct: None,
            last_call_me_maybe: None,
        });
    }

    /// Discovers our local candidate endpoints: the primary interface address
    /// toward `control_ip` plus, if `reflexive` is provided (from STUN), the
    /// server-reflexive endpoint. Both carry the UDP socket's port.
    pub fn set_local_endpoints(&mut self, control_ip: IpAddr, reflexive: Option<SocketAddr>) {
        let mut eps = Vec::new();
        if let Some(local_ip) = primary_local_ip(control_ip) {
            eps.push(SocketAddr::new(local_ip, self.port));
        }
        if let Some(r) = reflexive
            && !eps.contains(&r)
        {
            eps.push(r);
        }
        self.local_endpoints = eps;
    }

    /// Our candidate endpoints, to report to the control server so it
    /// propagates them (and our disco key) to peers.
    pub fn local_endpoints(&self) -> Vec<SocketAddr> {
        self.local_endpoints.clone()
    }

    /// The path a peer's traffic currently takes.
    pub fn path_for(&self, node: &NodePublic) -> PathKind {
        self.peers
            .get(node)
            .map(PeerPaths::path)
            .unwrap_or(PathKind::Relay)
    }

    /// Sends a WireGuard datagram to `node` over its best path: the verified
    /// direct endpoint if fresh, otherwise the DERP relay.
    pub async fn send_wg(&self, node: NodePublic, datagram: Vec<u8>) {
        match self.path_for(&node) {
            PathKind::Direct(addr) => {
                if let Err(e) = self.udp.send_to(&datagram, addr).await {
                    tracing::debug!(%addr, "magicsock: direct send failed, will fall back: {e}");
                    let _ = self.derp.send(node, datagram).await;
                }
            }
            PathKind::Relay => {
                let _ = self.derp.send(node, datagram).await;
            }
        }
    }

    /// Tells `node`, over DERP, our candidate endpoints so it can ping us.
    pub async fn send_call_me_maybe(&mut self, node: NodePublic) {
        if self.local_endpoints.is_empty() {
            tracing::debug!(peer = %node, "magicsock: no local endpoints, skipping call-me-maybe");
            return;
        }
        let Some(paths) = self.peers.get_mut(&node) else {
            return;
        };
        paths.last_call_me_maybe = Some(Instant::now());
        let disco_key = paths.disco_key;
        tracing::debug!(peer = %node, endpoints = ?self.local_endpoints, "magicsock: sending call-me-maybe");
        let msg = Message::CallMeMaybe {
            endpoints: self.local_endpoints.clone(),
        };
        let pkt = ts_disco::seal(&self.disco, &disco_key, &msg);
        let _ = self.derp.send(node, pkt).await;
    }

    /// Handles a disco payload that arrived over DERP (relayed by `from`).
    pub async fn on_derp_disco(&mut self, from: NodePublic, payload: &[u8]) {
        let _ = from;
        self.handle_disco(payload, DiscoVia::Derp).await;
    }

    /// Feeds an inbound UDP datagram: dispatches disco itself, or reports a
    /// WireGuard packet for the caller to decapsulate.
    pub async fn handle_udp(&mut self, payload: Vec<u8>, src: SocketAddr) -> UdpInput {
        if ts_disco::is_disco(&payload) {
            self.handle_disco(&payload, DiscoVia::Udp(src)).await;
            return UdpInput::HandledDisco;
        }
        match self.endpoint_to_node.get(&src) {
            Some(&peer) => UdpInput::WireGuard { peer, payload },
            None => UdpInput::Unknown,
        }
    }

    /// Heartbeat: re-ping each peer's current/candidate endpoints to keep NAT
    /// mappings open and detect path loss, and re-send call-me-maybe to peers
    /// that don't yet have a direct path (the first one can be dropped if it
    /// races the peer's DERP registration).
    pub async fn tick(&mut self) {
        // Re-send call-me-maybe to undiscovered peers, throttled.
        let need_cmm: Vec<NodePublic> = self
            .peers
            .iter()
            .filter(|(_, p)| {
                matches!(p.path(), PathKind::Relay)
                    && p.last_call_me_maybe
                        .is_none_or(|t| t.elapsed() >= CALL_ME_MAYBE_RESEND)
            })
            .map(|(n, _)| *n)
            .collect();
        for node in need_cmm {
            self.send_call_me_maybe(node).await;
        }

        let targets: Vec<(NodePublic, Vec<SocketAddr>)> = self
            .peers
            .iter()
            .map(|(node, p)| {
                let mut eps: Vec<SocketAddr> = p.candidates.clone();
                if let Some((addr, _)) = p.direct
                    && !eps.contains(&addr)
                {
                    eps.push(addr);
                }
                (*node, eps)
            })
            .collect();
        for (node, eps) in targets {
            for ep in eps {
                self.send_ping(node, ep).await;
            }
        }
    }

    async fn handle_disco(&mut self, payload: &[u8], via: DiscoVia) {
        let (sender_disco, msg) = match ts_disco::open(&self.disco, payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(
                    our_disco = %self.disco.public(),
                    src_disco = ?ts_disco::source_key(payload),
                    "magicsock: disco open failed: {e}"
                );
                return;
            }
        };
        let Some(&node) = self.disco_to_node.get(&sender_disco) else {
            tracing::debug!("magicsock: disco from unknown disco key, ignoring");
            return; // disco from an unknown peer
        };
        tracing::debug!(peer = %node, "magicsock: received disco message");
        match msg {
            Message::Ping { tx, .. } => self.on_ping(node, tx, via).await,
            Message::Pong { tx, .. } => self.on_pong(node, tx, via),
            Message::CallMeMaybe { endpoints } => self.on_call_me_maybe(node, endpoints).await,
        }
    }

    /// A peer pinged us. Pong back over the same path, and — if the ping came
    /// direct — treat its source as a candidate and probe it ourselves so the
    /// path is verified in both directions.
    async fn on_ping(&mut self, node: NodePublic, tx: TxId, via: DiscoVia) {
        let Some(paths) = self.peers.get(&node) else {
            return;
        };
        let disco_key = paths.disco_key;
        match via {
            DiscoVia::Udp(src) => {
                tracing::debug!(peer = %node, %src, "magicsock: got disco ping over UDP, ponging");
                let pong = Message::Pong { tx, src };
                let pkt = ts_disco::seal(&self.disco, &disco_key, &pong);
                let _ = self.udp.send_to(&pkt, src).await;
                self.add_candidate(node, src);
                self.send_ping(node, src).await;
            }
            DiscoVia::Derp => {
                // A ping relayed over DERP reports the sender's own view; we
                // reply over DERP so they learn we're reachable, but the
                // useful src address is unknown here.
                let pong = Message::Pong {
                    tx,
                    src: SocketAddr::from(([0, 0, 0, 0], 0)),
                };
                let pkt = ts_disco::seal(&self.disco, &disco_key, &pong);
                let _ = self.derp.send(node, pkt).await;
            }
        }
    }

    /// A pong arrived. If it answers one of our pings over UDP, the endpoint
    /// is a verified direct path — migrate this peer's traffic to it.
    fn on_pong(&mut self, node: NodePublic, tx: TxId, via: DiscoVia) {
        let DiscoVia::Udp(src) = via else {
            return;
        };
        let Some((expected_node, endpoint)) = self.pending.remove(&tx) else {
            return;
        };
        if expected_node != node {
            return;
        }
        if let Some(paths) = self.peers.get_mut(&node) {
            let was_relay = matches!(paths.path(), PathKind::Relay);
            paths.direct = Some((endpoint, Instant::now()));
            self.endpoint_to_node.insert(endpoint, node);
            self.endpoint_to_node.insert(src, node);
            if was_relay {
                tracing::info!(peer = %node, %endpoint, "magicsock: direct path UP (DERP→direct)");
            }
        }
    }

    /// A peer advertised its candidate endpoints; ping each to find a direct
    /// path.
    async fn on_call_me_maybe(&mut self, node: NodePublic, endpoints: Vec<SocketAddr>) {
        tracing::debug!(peer = %node, ?endpoints, "magicsock: got call-me-maybe, pinging endpoints");
        for ep in endpoints {
            self.add_candidate(node, ep);
            self.send_ping(node, ep).await;
        }
    }

    fn add_candidate(&mut self, node: NodePublic, ep: SocketAddr) {
        if ep.ip().is_unspecified() || ep.port() == 0 {
            return;
        }
        if let Some(paths) = self.peers.get_mut(&node)
            && !paths.candidates.contains(&ep)
        {
            paths.candidates.push(ep);
        }
        self.endpoint_to_node.entry(ep).or_insert(node);
    }

    /// Sends a disco ping to `endpoint` over UDP, recording the transaction so
    /// the pong can be matched.
    async fn send_ping(&mut self, node: NodePublic, endpoint: SocketAddr) {
        let Some(paths) = self.peers.get(&node) else {
            return;
        };
        let mut tx = [0u8; 12];
        OsRng.fill_bytes(&mut tx);
        let ping = Message::Ping {
            tx,
            node_key: NodePublic([0u8; 32]), // our node key is optional for disco
        };
        let pkt = ts_disco::seal(&self.disco, &paths.disco_key, &ping);
        self.pending.insert(tx, (node, endpoint));
        if let Err(e) = self.udp.send_to(&pkt, endpoint).await {
            tracing::trace!(%endpoint, "magicsock: ping send failed: {e}");
        }
    }
}

/// How a disco message reached us.
#[derive(Clone, Copy)]
enum DiscoVia {
    Udp(SocketAddr),
    /// Relayed over DERP (the sending peer is already known from the disco
    /// key, so no address is carried here).
    Derp,
}

/// The primary local IP the OS would use to reach `dst` — discovered by
/// connecting a throwaway UDP socket (no packets are sent) and reading its
/// local address.
fn primary_local_ip(dst: IpAddr) -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    sock.connect(SocketAddr::new(dst, 9)).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_unspecified() { None } else { Some(ip) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_local_ip_is_discovered() {
        // Toward a public-ish address, we should get some non-loopback local
        // IP (or at least a valid one). Just assert it doesn't panic and is
        // specified when it returns.
        if let Some(ip) = primary_local_ip(IpAddr::from([10, 0, 0, 1])) {
            assert!(!ip.is_unspecified());
        }
    }

    #[test]
    fn path_defaults_to_relay_and_expires() {
        let mut p = PeerPaths {
            disco_key: DiscoPublic([1; 32]),
            candidates: vec![],
            direct: None,
            last_call_me_maybe: None,
        };
        assert_eq!(p.path(), PathKind::Relay);
        let ep = SocketAddr::from(([10, 0, 0, 2], 41641));
        p.direct = Some((ep, Instant::now()));
        assert_eq!(p.path(), PathKind::Direct(ep));
        // A stale direct path falls back to relay.
        p.direct = Some((ep, Instant::now() - Duration::from_secs(60)));
        assert_eq!(p.path(), PathKind::Relay);
    }
}
