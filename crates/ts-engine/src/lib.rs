//! Engine: orchestrates control (netmap) → per-peer WireGuard → DERP
//! transport into a working, relayed data plane.
//!
//! Phase 3 is DERP-only: every WireGuard datagram rides a DERP frame keyed
//! by the peer's node public key — no direct paths, no path selection (that
//! is Phase 5's magicsock). There is no TUN device either (Phase 4): the
//! engine answers and originates ICMP echoes in userspace so two nodes can
//! prove relayed connectivity by pinging each other's `100.64.x.y`.
//!
//! Ports the engine speaks to (per the ports-and-adapters design):
//! `ts_control::ControlClient`, `ts_derp::DerpClient`, and the
//! `ts_wg::WgPeer` trait. Direct UDP and a real TUN slot into the same
//! seams in later phases.

pub mod icmp;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use ts_control::ControlClient;
use ts_derp::{DerpClient, DerpSender};
use ts_key::NodeState;
use ts_types::NodePublic;
use ts_types::tailcfg::{Hostinfo, MapResponse};
use ts_wg::{BoringWgPeer, WgAction, WgPeer};

/// How often WireGuard session timers are advanced.
const TICK_INTERVAL: Duration = Duration::from_millis(200);
/// Pending pings older than this are pruned.
const PING_EXPIRY: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Control(#[from] ts_control::ClientError),
    #[error("DERP error: {0}")]
    Derp(#[from] ts_derp::DerpError),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PingError {
    #[error("no peer with tailnet IP {0}")]
    UnknownPeer(Ipv4Addr),
    #[error("ping timed out")]
    Timeout,
    #[error("engine stopped")]
    EngineGone,
}

/// Configuration for [`Engine::start`].
pub struct EngineConfig {
    /// Control base URL, e.g. `http://127.0.0.1:8080`.
    pub control_url: String,
    /// DERP relay base URL, e.g. `http://127.0.0.1:8080`.
    pub derp_url: String,
    pub authkey: String,
    pub hostname: String,
}

/// A handle to a running engine.
#[derive(Clone)]
pub struct EngineHandle {
    cmd_tx: mpsc::Sender<Command>,
}

impl EngineHandle {
    /// Pings a peer by its tailnet IPv4 address, returning the round-trip
    /// time. The first ping to a peer includes the WireGuard handshake, so
    /// allow a generous timeout.
    pub async fn ping(&self, target: Ipv4Addr, timeout: Duration) -> Result<Duration, PingError> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Ping { target, reply })
            .await
            .map_err(|_| PingError::EngineGone)?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(PingError::EngineGone),
            Err(_) => Err(PingError::Timeout),
        }
    }

    /// Returns the tailnet IPs of currently known peers.
    pub async fn peer_ips(&self) -> Vec<Ipv4Addr> {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(Command::PeerIps { reply }).await.is_err() {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }
}

enum Command {
    Ping {
        target: Ipv4Addr,
        reply: oneshot::Sender<Result<Duration, PingError>>,
    },
    PeerIps {
        reply: oneshot::Sender<Vec<Ipv4Addr>>,
    },
}

struct PendingPing {
    started: Instant,
    reply: oneshot::Sender<Result<Duration, PingError>>,
}

/// The engine's owned state, driven by a single event-loop task.
pub struct Engine {
    state: NodeState,
    our_ips: Vec<Ipv4Addr>,
    derp: DerpClient,
    derp_tx: DerpSender,
    sessions: HashMap<NodePublic, BoringWgPeer>,
    ip_to_key: HashMap<Ipv4Addr, NodePublic>,
    next_index: u32,
    ping_counter: u16,
    pending: HashMap<u16, PendingPing>,
    netmap_rx: mpsc::UnboundedReceiver<MapResponse>,
    cmd_rx: mpsc::Receiver<Command>,
}

impl Engine {
    /// Registers with the control server, connects to DERP, and spawns the
    /// event loop. Returns a handle for pinging peers.
    pub async fn start(
        config: EngineConfig,
        state: NodeState,
    ) -> Result<EngineHandle, EngineError> {
        let node_key = state.node.public();
        let disco_key = state.disco.public();

        let hostinfo = Hostinfo {
            ipn_version: format!("tailscale-rs-engine-{}", env!("CARGO_PKG_VERSION")),
            hostname: config.hostname.clone(),
            os: std::env::consts::OS.to_string(),
            routable_ips: Vec::new(),
        };

        let control =
            ControlClient::connect(&config.control_url, state.machine.clone(), hostinfo).await?;
        control.register(node_key, &config.authkey).await?;
        tracing::info!("engine: registered with control server");

        // Stream the netmap into an unbounded channel (the poll handler is
        // sync; unbounded send never blocks it).
        let (netmap_tx, netmap_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let r = control
                .poll_netmap(node_key, disco_key, move |resp| {
                    if netmap_tx.send(resp).is_err() {
                        std::ops::ControlFlow::Break(())
                    } else {
                        std::ops::ControlFlow::Continue(())
                    }
                })
                .await;
            if let Err(e) = r {
                tracing::warn!("engine: netmap poll ended: {e}");
            }
        });

        let derp = DerpClient::connect(&config.derp_url, &state.node).await?;
        let derp_tx = derp.sender();
        tracing::info!(server_key = %derp.server_key(), "engine: connected to DERP");

        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let engine = Engine {
            state,
            our_ips: Vec::new(),
            derp,
            derp_tx,
            sessions: HashMap::new(),
            ip_to_key: HashMap::new(),
            next_index: 1,
            ping_counter: 0,
            pending: HashMap::new(),
            netmap_rx,
            cmd_rx,
        };
        tokio::spawn(engine.run());
        Ok(EngineHandle { cmd_tx })
    }

    async fn run(mut self) {
        let mut tick = tokio::time::interval(TICK_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                Some(resp) = self.netmap_rx.recv() => self.apply_netmap(resp),
                maybe_pkt = self.derp.recv() => match maybe_pkt {
                    Some(pkt) => self.on_derp_packet(pkt.peer, pkt.payload).await,
                    None => {
                        tracing::warn!("engine: DERP connection closed; stopping");
                        break;
                    }
                },
                Some(cmd) = self.cmd_rx.recv() => self.on_command(cmd).await,
                _ = tick.tick() => self.on_tick().await,
                else => break,
            }
        }
    }

    /// Applies a netmap frame: learns our own IPs and ensures a WireGuard
    /// session + IP mapping for every peer.
    fn apply_netmap(&mut self, resp: MapResponse) {
        if let Some(node) = resp.node {
            for ip in ipv4_addrs(&node.addresses) {
                if !self.our_ips.contains(&ip) {
                    self.our_ips.push(ip);
                    tracing::info!(%ip, "engine: local tailnet address");
                }
            }
        }
        let mut incoming = Vec::new();
        if let Some(peers) = resp.peers {
            incoming.extend(peers);
        }
        if let Some(changed) = resp.peers_changed {
            incoming.extend(changed);
        }
        for peer in incoming {
            let Some(key) = peer.key else { continue };
            self.ensure_session(key);
            for ip in ipv4_addrs(&peer.addresses) {
                self.ip_to_key.insert(ip, key);
            }
        }
    }

    fn ensure_session(&mut self, key: NodePublic) -> &mut BoringWgPeer {
        if !self.sessions.contains_key(&key) {
            let index = self.next_index;
            self.next_index += 1;
            self.sessions
                .insert(key, BoringWgPeer::new(&self.state.node, key, index));
            tracing::debug!(peer = %key, "engine: created WireGuard session");
        }
        self.sessions.get_mut(&key).expect("just inserted")
    }

    /// Handles a WireGuard datagram relayed to us from `peer`.
    async fn on_derp_packet(&mut self, peer: NodePublic, payload: Vec<u8>) {
        let actions = self.ensure_session(peer).decapsulate(&payload);
        for action in actions {
            match action {
                WgAction::ToPeer(dg) => self.send_to_peer(peer, dg).await,
                WgAction::ToLocal(ip_pkt) => self.handle_ip(peer, ip_pkt).await,
            }
        }
    }

    /// Handles a decrypted inbound IP packet: answer ICMP echo requests,
    /// resolve pending pings on echo replies.
    async fn handle_ip(&mut self, peer: NodePublic, ip_pkt: Vec<u8>) {
        let Some(ip) = icmp::parse_ipv4(&ip_pkt) else {
            return;
        };
        if ip.protocol != icmp::PROTO_ICMP {
            return;
        }
        let Some(echo) = ip_pkt.get(ip.payload_offset..).and_then(icmp::parse_echo) else {
            return;
        };

        if echo.is_reply {
            if let Some(p) = self.pending.remove(&echo.id) {
                let _ = p.reply.send(Ok(p.started.elapsed()));
            }
            return;
        }

        // Echo request addressed to one of our IPs → reply through the tunnel.
        if self.our_ips.contains(&ip.dst)
            && let Some(reply) = icmp::build_echo_reply(&ip_pkt)
        {
            let actions = self.ensure_session(peer).encapsulate(&reply);
            for a in actions {
                if let WgAction::ToPeer(dg) = a {
                    self.send_to_peer(peer, dg).await;
                }
            }
        }
    }

    async fn on_command(&mut self, cmd: Command) {
        match cmd {
            Command::Ping { target, reply } => self.start_ping(target, reply).await,
            Command::PeerIps { reply } => {
                let _ = reply.send(self.ip_to_key.keys().copied().collect());
            }
        }
    }

    async fn start_ping(
        &mut self,
        target: Ipv4Addr,
        reply: oneshot::Sender<Result<Duration, PingError>>,
    ) {
        let Some(&key) = self.ip_to_key.get(&target) else {
            let _ = reply.send(Err(PingError::UnknownPeer(target)));
            return;
        };
        let Some(src) = self.our_ips.first().copied() else {
            let _ = reply.send(Err(PingError::UnknownPeer(target)));
            return;
        };

        self.ping_counter = self.ping_counter.wrapping_add(1);
        let id = self.ping_counter;
        let request = icmp::build_echo_request(src, target, id, 1, b"tailscale-rs ping");
        self.pending.insert(
            id,
            PendingPing {
                started: Instant::now(),
                reply,
            },
        );

        let actions = self.ensure_session(key).encapsulate(&request);
        for a in actions {
            if let WgAction::ToPeer(dg) = a {
                self.send_to_peer(key, dg).await;
            }
        }
    }

    /// Advances every WireGuard session's timers (handshakes/keepalives) and
    /// prunes expired pings.
    async fn on_tick(&mut self) {
        let keys: Vec<NodePublic> = self.sessions.keys().copied().collect();
        for key in keys {
            let actions = self
                .sessions
                .get_mut(&key)
                .map(|s| s.tick())
                .unwrap_or_default();
            for a in actions {
                if let WgAction::ToPeer(dg) = a {
                    self.send_to_peer(key, dg).await;
                }
            }
        }
        self.pending
            .retain(|_, p| p.started.elapsed() < PING_EXPIRY);
    }

    async fn send_to_peer(&self, peer: NodePublic, datagram: Vec<u8>) {
        if let Err(e) = self.derp_tx.send(peer, datagram).await {
            tracing::debug!(peer = %peer, "engine: DERP send failed: {e}");
        }
    }
}

/// Extracts IPv4 addresses from a list of tailnet prefixes.
fn ipv4_addrs(prefixes: &[ts_types::IpPrefix]) -> Vec<Ipv4Addr> {
    prefixes
        .iter()
        .filter_map(|p| match p.addr {
            std::net::IpAddr::V4(v4) => Some(v4),
            std::net::IpAddr::V6(_) => None,
        })
        .collect()
}
