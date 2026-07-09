//! Engine: orchestrates control (netmap) → per-peer WireGuard →
//! magicsock (direct/DERP) → TUN into a working data plane.
//!
//! WireGuard datagrams are handed to `ts_magicsock`, which sends them over a
//! verified direct UDP path when one exists (Phase 5) or the DERP relay
//! otherwise (Phase 3); decrypted packets go to a real TUN device (Phase 4)
//! or, without one, a userspace ICMP responder. Every layer is reached
//! through a port (`ts_control::ControlClient`, `ts_derp::DerpClient`,
//! `ts_magicsock::MagicSock`, `ts_wg::WgPeer`, `ts_tun::Tun`) so the domain
//! logic stays testable and the adapters are swappable.

pub mod icmp;
mod l4;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};
use ts_control::ControlClient;
use ts_derp::{DerpClient, DerpSender};
use ts_filter::Filter;
use ts_key::NodeState;
use ts_magicsock::{MagicSock, PathKind, UdpInput};
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
    #[error("magicsock I/O error: {0}")]
    Io(#[from] std::io::Error),
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
    /// Enable direct-path discovery (magicsock/disco). When true the engine
    /// binds a UDP socket, runs disco, and upgrades peers from DERP to direct
    /// paths (Phase 5). When false, all traffic stays on DERP (Phase 3/4).
    pub enable_direct: bool,
    /// Optional `host:port` STUN server for reflexive-endpoint discovery
    /// (needed for NAT traversal; unnecessary on a flat network).
    pub stun_server: Option<String>,
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

    /// A snapshot of tailnet status (self + peers + connection state),
    /// LocalAPI-compatible.
    pub async fn status(&self) -> Option<ts_types::Status> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx.send(Command::Status { reply }).await.ok()?;
        rx.await.ok()
    }

    /// Sets whether the data plane is running (`up`/`down`).
    pub async fn set_want_running(&self, want: bool) -> bool {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(Command::SetWantRunning { want, reply })
            .await
            .is_err()
        {
            return false;
        }
        rx.await.is_ok()
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
    Status {
        reply: oneshot::Sender<ts_types::Status>,
    },
    SetWantRunning {
        want: bool,
        reply: oneshot::Sender<()>,
    },
}

struct PendingPing {
    started: Instant,
    reply: oneshot::Sender<Result<Duration, PingError>>,
}

/// Netmap-derived metadata about a peer, for status reporting.
#[derive(Default, Clone)]
struct PeerMeta {
    stable_id: ts_types::StableNodeID,
    host_name: String,
    dns_name: String,
    os: String,
    ips: Vec<std::net::IpAddr>,
    allowed_ips: Vec<ts_types::IpPrefix>,
    user_id: ts_types::UserID,
    online: bool,
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
    /// Direct-path multiplexer (Phase 5). When present, WireGuard datagrams
    /// go through it (direct or DERP); when absent, straight to DERP.
    magicsock: Option<MagicSock>,
    /// The disco key we've registered per peer, to detect changes (Headscale
    /// may send a zero disco key until the peer reports endpoints).
    peer_disco: HashMap<NodePublic, ts_types::DiscoPublic>,
    /// Our own hostname, for status.
    hostname: String,
    /// Our own DNS name (from the netmap self node).
    self_dns_name: String,
    /// Our own stable node ID (from the netmap self node).
    self_stable_id: ts_types::StableNodeID,
    /// Per-peer netmap metadata, for status.
    peers_meta: HashMap<NodePublic, PeerMeta>,
    /// User profiles by ID (from the netmap), for status ownership column.
    users: HashMap<ts_types::UserID, ts_types::UserProfile>,
    /// Whether the data plane is active (`tailscale up`/`down`). When false,
    /// inbound and outbound tunnel traffic is dropped and status reports
    /// `Stopped`.
    want_running: bool,
    /// Inbound ACL enforcement, compiled from the netmap packet filter.
    /// Starts permissive so startup traffic isn't black-holed before the
    /// first netmap arrives.
    filter: Filter,
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

        let derp = DerpClient::connect(&config.derp_url, &state.node).await?;
        let derp_tx = derp.sender();
        tracing::info!(server_key = %derp.server_key(), "engine: connected to DERP");

        // Resolve the control server's IP for local-endpoint discovery.
        let control_ip = resolve_host_ip(&config.control_url);

        // Bring up the direct-path multiplexer, if enabled. Do this *before*
        // starting the netmap poll so we can report our endpoints in the map
        // request — the control server needs them to propagate our disco key
        // to peers.
        let magicsock = if config.enable_direct {
            let mut ms = MagicSock::new(0, state.disco.clone(), derp.sender()).await?;
            let reflexive = match &config.stun_server {
                Some(s) => ms.discover_reflexive(s).await,
                None => None,
            };
            if let Some(cip) = control_ip {
                ms.set_local_endpoints(cip, reflexive);
            }
            tracing::info!(
                udp_port = ms.port(),
                "engine: magicsock up (direct paths enabled)"
            );
            Some(ms)
        } else {
            None
        };
        let endpoints = magicsock
            .as_ref()
            .map(|m| m.local_endpoints())
            .unwrap_or_default();

        // Report our disco key + endpoints via a lite map request so the
        // control server persists them and propagates our disco key to peers
        // (a prerequisite for NAT traversal). Headscale ignores these on the
        // streaming poll.
        if !endpoints.is_empty() {
            if let Err(e) = control
                .update_endpoints(node_key, disco_key, endpoints.clone())
                .await
            {
                tracing::warn!("engine: endpoint update failed: {e}");
            } else {
                tracing::info!(?endpoints, "engine: reported endpoints to control");
            }
        }

        // Stream the netmap into an unbounded channel (the poll handler is
        // sync; unbounded send never blocks it).
        let (netmap_tx, netmap_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let r = control
                .poll_netmap(node_key, disco_key, endpoints, move |resp| {
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
            magicsock,
            peer_disco: HashMap::new(),
            hostname: config.hostname,
            self_dns_name: String::new(),
            self_stable_id: ts_types::StableNodeID::default(),
            peers_meta: HashMap::new(),
            users: HashMap::new(),
            want_running: true,
            filter: Filter::allow_all(),
        };
        tokio::spawn(engine.run());
        Ok(EngineHandle { cmd_tx })
    }

    async fn run(mut self) {
        let mut tick = tokio::time::interval(TICK_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            // Clone the TUN handle and UDP socket (cheap Arc, or None) so the
            // read futures borrow locals rather than `self`, leaving the other
            // branches free to take `&mut self`.
            let tun = self.tun.clone();
            let udp = self.magicsock.as_ref().map(|m| m.udp());
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
                res = recv_udp(&udp), if udp.is_some() => match res {
                    Some(Ok((buf, src))) => self.on_udp_packet(buf, src).await,
                    Some(Err(e)) => tracing::trace!("engine: UDP read error: {e}"),
                    None => {}
                },
                else => break,
            }
        }
    }

    /// Handles an inbound UDP datagram: magicsock dispatches disco itself and
    /// reports WireGuard packets for us to decapsulate (their WG output may in
    /// turn go back over the newly discovered direct path).
    async fn on_udp_packet(&mut self, buf: Vec<u8>, src: SocketAddr) {
        let input = match &mut self.magicsock {
            Some(ms) => ms.handle_udp(buf, src).await,
            None => return,
        };
        if let UdpInput::WireGuard { peer, payload } = input {
            self.deliver_wg(peer, &payload).await;
        }
    }

    /// Handles an outbound IP packet the kernel routed into our TUN: find the
    /// peer owning the destination address, encrypt, and relay it.
    async fn on_tun_packet(&mut self, mut packet: Vec<u8>) {
        if !self.want_running {
            return; // data plane stopped (`tailscale down`)
        }
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
        if let Some(node) = &resp.node {
            for ip in ipv4_addrs(&node.addresses) {
                if !self.our_ips.contains(&ip) {
                    self.our_ips.push(ip);
                    tracing::info!(%ip, "engine: local tailnet address");
                    self.ensure_tun(ip);
                }
            }
            if !node.name.is_empty() {
                self.self_dns_name = node.name.clone();
            }
            if !node.stable_id.0.is_empty() {
                self.self_stable_id = node.stable_id.clone();
            }
            if let (Some(ip), false) = (self.our_ips.first().copied(), node.name.is_empty())
                && self.record_dns(ip, &node.name)
            {
                dns_changed = true;
            }
        }

        // Accumulate user profiles (delta stream: absent means unchanged).
        if let Some(profiles) = &resp.user_profiles {
            for p in profiles {
                self.users.insert(p.id, p.clone());
            }
        }

        // Recompile the packet filter when the netmap carries one. Modern
        // Headscale sends the named `PacketFilters` map; older servers the
        // flat `PacketFilter`. The effective ruleset is the union.
        if resp.packet_filter.is_some() || resp.packet_filters.is_some() {
            let mut rules = Vec::new();
            if let Some(pf) = &resp.packet_filter {
                rules.extend(pf.iter().cloned());
            }
            if let Some(pfs) = &resp.packet_filters {
                for set in pfs.values() {
                    rules.extend(set.iter().cloned());
                }
            }
            self.filter = Filter::new(&rules);
            tracing::info!(
                rules = self.filter.rule_count(),
                "engine: packet filter updated"
            );
        }

        let mut incoming = Vec::new();
        if let Some(peers) = resp.peers {
            incoming.extend(peers);
        }
        if let Some(changed) = resp.peers_changed {
            incoming.extend(changed);
        }
        let mut new_disco: Vec<(NodePublic, ts_types::DiscoPublic)> = Vec::new();
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
            // Capture netmap metadata for status reporting.
            let meta = self.peers_meta.entry(key).or_default();
            if !peer.stable_id.0.is_empty() {
                meta.stable_id = peer.stable_id.clone();
            }
            if !peer.name.is_empty() {
                meta.dns_name = peer.name.clone();
            }
            if let Some(hi) = &peer.hostinfo {
                if !hi.hostname.is_empty() {
                    meta.host_name = hi.hostname.clone();
                }
                if !hi.os.is_empty() {
                    meta.os = hi.os.clone();
                }
            }
            meta.ips = peer.addresses.iter().map(|p| p.addr).collect();
            meta.allowed_ips = peer.allowed_ips.clone();
            meta.user_id = peer.user;
            if let Some(online) = peer.online {
                meta.online = online;
            }
            // Register/update the peer's disco key when it's non-zero and has
            // changed (Headscale sends a zero key until the peer reports
            // endpoints).
            if let Some(disco) = peer.disco_key
                && disco != ts_types::DiscoPublic([0u8; 32])
                && self.peer_disco.get(&key) != Some(&disco)
            {
                self.peer_disco.insert(key, disco);
                new_disco.push((key, disco));
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

        // Register each peer's disco key with magicsock and invite it to
        // ping us, kicking off direct-path discovery (Phase 5).
        if let Some(ms) = &mut self.magicsock {
            for (node, disco) in &new_disco {
                tracing::debug!(peer = %node, peer_disco = %disco, our_disco = %self.state.disco.public(), "engine: registering peer disco key");
                ms.add_peer(*node, *disco);
            }
            for (node, _) in &new_disco {
                ms.send_call_me_maybe(*node).await;
            }
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

    /// Handles a packet relayed to us over DERP from `peer`: a disco control
    /// message (handed to magicsock) or a WireGuard datagram.
    async fn on_derp_packet(&mut self, peer: NodePublic, payload: Vec<u8>) {
        if ts_disco::is_disco(&payload) {
            if let Some(ms) = &mut self.magicsock {
                ms.on_derp_disco(peer, &payload).await;
            }
            return;
        }
        self.deliver_wg(peer, &payload).await;
    }

    /// Decapsulates a WireGuard datagram from `peer` (arriving via DERP or a
    /// direct UDP path) and dispatches its output: reply datagrams back to the
    /// peer, decrypted IP packets to the TUN (or userspace ICMP).
    async fn deliver_wg(&mut self, peer: NodePublic, payload: &[u8]) {
        let actions = self.ensure_session(peer).decapsulate(payload);
        for action in actions {
            match action {
                // Always answer the peer at the WireGuard layer (handshakes,
                // keepalives) so the tunnel survives a `down`; only inbound
                // *user* traffic is gated below.
                WgAction::ToPeer(dg) => self.send_to_peer(peer, dg).await,
                WgAction::ToLocal(_) if !self.want_running => {}
                WgAction::ToLocal(ip_pkt) => {
                    // Enforce the inbound ACL: drop packets the packet filter
                    // doesn't permit (Phase 6). Non-IPv4 is passed through to
                    // the OS (we don't yet parse IPv6 for filtering).
                    if !self.filter_allows_inbound(&ip_pkt) {
                        tracing::debug!("engine: inbound packet denied by ACL");
                        continue;
                    }
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

    /// Checks a decrypted inbound IPv4 packet against the packet filter.
    /// Non-IPv4 packets are allowed (IPv6 filtering isn't implemented yet).
    fn filter_allows_inbound(&self, ip_pkt: &[u8]) -> bool {
        let Some(view) = icmp::parse_ipv4(ip_pkt) else {
            return true; // not IPv4 (or too short): don't filter here
        };
        // Destination L4 port for port-bearing protocols (TCP/UDP/SCTP live at
        // bytes 2..4 of the transport header); 0 for port-less protocols.
        let dst_port = match view.protocol {
            6 | 17 | 132 => ip_pkt
                .get(view.payload_offset + 2..view.payload_offset + 4)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                .unwrap_or(0),
            _ => 0,
        };
        self.filter.allows(
            IpAddr::V4(view.src),
            IpAddr::V4(view.dst),
            view.protocol,
            dst_port,
        )
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
            Command::Status { reply } => {
                let _ = reply.send(self.build_status());
            }
            Command::SetWantRunning { want, reply } => {
                if self.want_running != want {
                    self.want_running = want;
                    tracing::info!(want_running = want, "engine: WantRunning changed");
                }
                let _ = reply.send(());
            }
        }
    }

    /// Assembles a LocalAPI-compatible [`ts_types::Status`] snapshot from the
    /// current netmap-derived metadata plus live path state from magicsock.
    fn build_status(&self) -> ts_types::Status {
        use ts_types::{PeerStatus, Status};

        let version = format!("tailscale-rs-{}", env!("CARGO_PKG_VERSION"));
        let backend_state = if self.want_running {
            "Running"
        } else {
            "Stopped"
        }
        .to_string();

        let self_ips: Vec<IpAddr> = self.our_ips.iter().map(|v4| IpAddr::V4(*v4)).collect();
        let self_status = PeerStatus {
            id: self.self_stable_id.clone(),
            public_key: Some(self.state.node.public()),
            host_name: self.hostname.clone(),
            dns_name: self.self_dns_name.clone(),
            os: std::env::consts::OS.to_string(),
            tailscale_ips: self_ips.clone(),
            online: true,
            in_network_map: true,
            in_magic_sock: true,
            in_engine: true,
            ..Default::default()
        };

        let mut peer_map = std::collections::BTreeMap::new();
        for (key, meta) in &self.peers_meta {
            let path = self.path_for(key);
            let (cur_addr, relay) = match path {
                PathKind::Direct(addr) => (addr.to_string(), String::new()),
                PathKind::Relay => (String::new(), "derp".to_string()),
            };
            let active = self.sessions.contains_key(key);
            let ps = PeerStatus {
                id: meta.stable_id.clone(),
                public_key: Some(*key),
                host_name: meta.host_name.clone(),
                dns_name: meta.dns_name.clone(),
                os: meta.os.clone(),
                user_id: meta.user_id,
                tailscale_ips: meta.ips.clone(),
                allowed_ips: meta.allowed_ips.clone(),
                cur_addr,
                relay,
                online: meta.online,
                active: active && meta.online,
                in_network_map: true,
                in_magic_sock: self.peer_disco.contains_key(key),
                in_engine: active,
                ..Default::default()
            };
            peer_map.insert(*key, ps);
        }

        Status {
            version,
            tun: self.tun.is_some(),
            backend_state,
            have_node_key: true,
            tailscale_ips: self_ips,
            self_: Some(self_status),
            peer: peer_map,
            user: self.users.iter().map(|(k, v)| (*k, v.clone())).collect(),
            ..Default::default()
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

        // Heartbeat disco: keep NAT mappings open and detect path loss.
        if let Some(ms) = &mut self.magicsock {
            ms.tick().await;
        }
    }

    /// Sends a WireGuard datagram to `peer` over its best path: through
    /// magicsock (direct if a path is up, else DERP) when direct paths are
    /// enabled, otherwise straight over DERP.
    async fn send_to_peer(&self, peer: NodePublic, datagram: Vec<u8>) {
        match &self.magicsock {
            Some(ms) => ms.send_wg(peer, datagram).await,
            None => {
                if let Err(e) = self.derp_tx.send(peer, datagram).await {
                    tracing::debug!(peer = %peer, "engine: DERP send failed: {e}");
                }
            }
        }
    }

    /// The current path (direct or relay) for a peer, for status reporting.
    pub fn path_for(&self, peer: &NodePublic) -> PathKind {
        self.magicsock
            .as_ref()
            .map(|m| m.path_for(peer))
            .unwrap_or(PathKind::Relay)
    }
}

/// Awaits the next UDP datagram from the magicsock socket, or never resolves
/// when direct paths are disabled.
async fn recv_udp(udp: &Option<Arc<UdpSocket>>) -> Option<std::io::Result<(Vec<u8>, SocketAddr)>> {
    match udp {
        Some(sock) => {
            let mut buf = vec![0u8; 65_536];
            let r = sock.recv_from(&mut buf).await.map(|(n, src)| {
                buf.truncate(n);
                (buf, src)
            });
            Some(r)
        }
        None => std::future::pending().await,
    }
}

/// Resolves a `http://host:port` control URL's host to an IP for
/// local-endpoint discovery.
fn resolve_host_ip(control_url: &str) -> Option<IpAddr> {
    use std::net::ToSocketAddrs;
    let host = control_url
        .strip_prefix("http://")
        .or_else(|| control_url.strip_prefix("https://"))
        .unwrap_or(control_url);
    let authority = host.split(['/', '?']).next().unwrap_or(host);
    // Ensure there's a port for ToSocketAddrs.
    let with_port = if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    with_port.to_socket_addrs().ok()?.next().map(|sa| sa.ip())
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
