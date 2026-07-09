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
mod l4;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use ts_control::ControlClient;
use ts_derp::{DerpClient, DerpSender};
use ts_key::NodeState;
use ts_tun::Tun;
use ts_types::NodePublic;
use ts_types::tailcfg::{Hostinfo, MapResponse};
use ts_wg::{BoringWgPeer, WgAction, WgPeer};

/// How often WireGuard session timers are advanced.
const TICK_INTERVAL: Duration = Duration::from_millis(200);
/// Pending pings older than this are pruned.
const PING_EXPIRY: Duration = Duration::from_secs(15);
/// Prefix length of the tailnet CGNAT range (`100.64.0.0/10`). Assigning the
/// TUN address with this prefix installs the connected route for the whole
/// range, so peers are reachable without an explicit route command.
const TAILNET_PREFIX_LEN: u8 = 10;

/// Awaits the next TUN packet, or never resolves when there is no device.
async fn recv_tun(tun: &Option<Tun>) -> Option<std::io::Result<Vec<u8>>> {
    match tun {
        Some(t) => Some(t.recv().await),
        None => std::future::pending().await,
    }
}

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
    /// If set, create a TUN device with this name once the netmap yields our
    /// tailnet IP; real OS traffic then rides the tunnel (Phase 4). If
    /// `None`, the engine runs the userspace ICMP data plane (Phase 3), which
    /// needs no root.
    pub tun_name: Option<String>,
    /// If set, write MagicDNS peer name→IP mappings into this hosts file
    /// (managed block).
    pub magic_dns_hosts: Option<std::path::PathBuf>,
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
    /// TUN device (Phase 4). Created lazily once we learn our tailnet IP.
    tun: Option<Tun>,
    tun_name: Option<String>,
    /// MagicDNS: hosts file to manage, and the peer name→IP mappings we've
    /// written (to avoid rewriting when unchanged).
    magic_dns_hosts: Option<std::path::PathBuf>,
    dns_entries: std::collections::BTreeMap<Ipv4Addr, String>,
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
            tun: None,
            tun_name: config.tun_name,
            magic_dns_hosts: config.magic_dns_hosts,
            dns_entries: std::collections::BTreeMap::new(),
        };
        tokio::spawn(engine.run());
        Ok(EngineHandle { cmd_tx })
    }

    async fn run(mut self) {
        let mut tick = tokio::time::interval(TICK_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            // Clone the TUN handle (cheap Arc, or None) so the read future
            // borrows a local rather than `self`, leaving the other branches
            // free to take `&mut self`.
            let tun = self.tun.clone();
            tokio::select! {
                Some(resp) = self.netmap_rx.recv() => self.apply_netmap(resp).await,
                maybe_pkt = self.derp.recv() => match maybe_pkt {
                    Some(pkt) => self.on_derp_packet(pkt.peer, pkt.payload).await,
                    None => {
                        tracing::warn!("engine: DERP connection closed; stopping");
                        break;
                    }
                },
                Some(cmd) = self.cmd_rx.recv() => self.on_command(cmd).await,
                _ = tick.tick() => self.on_tick().await,
                res = recv_tun(&tun), if tun.is_some() => match res {
                    Some(Ok(pkt)) => self.on_tun_packet(pkt).await,
                    Some(Err(e)) => tracing::warn!("engine: TUN read error: {e}"),
                    None => {}
                },
                else => break,
            }
        }
    }

    /// Handles an outbound IP packet the kernel routed into our TUN: find the
    /// peer owning the destination address, encrypt, and relay it.
    async fn on_tun_packet(&mut self, mut packet: Vec<u8>) {
        let Some(ip) = icmp::parse_ipv4(&packet) else {
            return; // IPv6 / non-IP: not handled in Phase 4
        };
        let Some(&key) = self.ip_to_key.get(&ip.dst) else {
            tracing::trace!(dst = %ip.dst, "engine: no peer for TUN packet, dropping");
            return;
        };
        // Complete any offloaded (CHECKSUM_PARTIAL) TCP/UDP checksum before
        // the packet leaves this host, or the peer's stack will drop it.
        l4::fix_ipv4_transport_checksum(&mut packet);
        let actions = self.ensure_session(key).encapsulate(&packet);
        for a in actions {
            if let WgAction::ToPeer(dg) = a {
                self.send_to_peer(key, dg).await;
            }
        }
    }

    /// Applies a netmap frame: learns our own IPs (creating the TUN device on
    /// first sight), ensures a WireGuard session + IP mapping for every peer,
    /// and refreshes MagicDNS.
    async fn apply_netmap(&mut self, resp: MapResponse) {
        let mut dns_changed = false;
        let mut new_peers: Vec<NodePublic> = Vec::new();
        if let Some(node) = resp.node {
            for ip in ipv4_addrs(&node.addresses) {
                if !self.our_ips.contains(&ip) {
                    self.our_ips.push(ip);
                    tracing::info!(%ip, "engine: local tailnet address");
                    self.ensure_tun(ip);
                }
            }
            if let (Some(ip), false) = (self.our_ips.first().copied(), node.name.is_empty())
                && self.record_dns(ip, &node.name)
            {
                dns_changed = true;
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
            if !self.sessions.contains_key(&key) {
                new_peers.push(key);
            }
            self.ensure_session(key);
            let peer_ips = ipv4_addrs(&peer.addresses);
            for ip in &peer_ips {
                self.ip_to_key.insert(*ip, key);
            }
            if let (Some(ip), false) = (peer_ips.first().copied(), peer.name.is_empty())
                && self.record_dns(ip, &peer.name)
            {
                dns_changed = true;
            }
        }

        if dns_changed {
            self.write_magic_dns();
        }

        // Proactively start the WireGuard handshake with each newly learned
        // peer so the first real connection doesn't have to wait for (and
        // possibly drop packets during) the handshake — matching tailscaled,
        // which handshakes on learning a peer rather than on first traffic.
        for key in new_peers {
            self.initiate_handshake(key).await;
        }
    }

    /// Kicks off a WireGuard handshake with `key` by encapsulating an empty
    /// packet (boringtun emits a handshake initiation when no session
    /// exists) and relaying the result over DERP.
    async fn initiate_handshake(&mut self, key: NodePublic) {
        let actions = self.ensure_session(key).encapsulate(&[]);
        for a in actions {
            if let WgAction::ToPeer(dg) = a {
                self.send_to_peer(key, dg).await;
            }
        }
    }

    /// Creates the TUN device (Phase 4) if configured and not yet created,
    /// bound to our tailnet IP with the `100.64.0.0/10` connected route.
    fn ensure_tun(&mut self, ip: Ipv4Addr) {
        if self.tun.is_some() {
            return;
        }
        let Some(name) = self.tun_name.clone() else {
            return;
        };
        match Tun::create(&name, ip, TAILNET_PREFIX_LEN, ts_tun::DEFAULT_MTU) {
            Ok(tun) => {
                tracing::info!(device = %name, %ip, "engine: TUN device up");
                self.tun = Some(tun);
            }
            Err(e) => tracing::error!("engine: failed to create TUN {name}: {e}"),
        }
    }

    /// Records a name→IP mapping; returns true if it changed.
    fn record_dns(&mut self, ip: Ipv4Addr, dns_name: &str) -> bool {
        if self.magic_dns_hosts.is_none() {
            return false;
        }
        let fqdn = dns_name.trim_end_matches('.').to_string();
        match self.dns_entries.get(&ip) {
            Some(existing) if *existing == fqdn => false,
            _ => {
                self.dns_entries.insert(ip, fqdn);
                true
            }
        }
    }

    /// Rewrites the managed MagicDNS block in the configured hosts file.
    fn write_magic_dns(&self) {
        let Some(path) = &self.magic_dns_hosts else {
            return;
        };
        let entries: Vec<ts_tun::magicdns::HostEntry> = self
            .dns_entries
            .iter()
            .filter_map(|(ip, fqdn)| ts_tun::magicdns::HostEntry::from_dns_name(*ip, fqdn))
            .collect();
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        let merged = ts_tun::magicdns::merge_into(&existing, &entries);
        if let Err(e) = std::fs::write(path, merged) {
            tracing::warn!("engine: failed to write MagicDNS hosts {path:?}: {e}");
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
                WgAction::ToLocal(ip_pkt) => {
                    if let Some(tun) = &self.tun {
                        // Phase 4: hand the decrypted packet to the OS stack.
                        if let Err(e) = tun.send(&ip_pkt).await {
                            tracing::warn!("engine: TUN write error: {e}");
                        }
                    } else {
                        // Phase 3 fallback: userspace ICMP echo.
                        self.handle_ip(peer, ip_pkt).await;
                    }
                }
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
