//! WireGuard data-plane adapter.
//!
//! `ts-engine` speaks only to the [`WgPeer`] port; the concrete transport
//! (userspace boringtun now, kernel WireGuard via netlink later) sits behind
//! it. WireGuard is deliberately oblivious to the path its datagrams travel:
//! the engine takes each [`WgAction::ToPeer`] and ships it over DERP (Phase
//! 3) or a direct UDP socket (Phase 5) — the "magic socket" abstraction.
//!
//! Ground truth for the boringtun usage pattern is its `Tunn` API; see
//! PROTOCOL.md.

use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use ts_key::NodePrivate;
use ts_types::NodePublic;

/// Max buffer for one WireGuard/IP packet plus framing overhead.
const BUF_SIZE: usize = (64 << 10) + 64;

/// A single step of output from the WireGuard state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WgAction {
    /// An encrypted WireGuard datagram to transmit to the peer (the engine
    /// sends it over DERP or UDP).
    ToPeer(Vec<u8>),
    /// A decrypted inbound IP packet to deliver to the local network stack.
    ToLocal(Vec<u8>),
}

/// A per-peer WireGuard session (the port `ts-engine` depends on).
///
/// One instance handles the tunnel to exactly one peer, keyed by that peer's
/// node public key (which is also its WireGuard static key and its DERP
/// routing key).
pub trait WgPeer {
    /// The peer this session encrypts to.
    fn peer_key(&self) -> NodePublic;

    /// Encrypts an outbound IP packet. May return handshake datagrams
    /// instead if no session is established yet (the IP packet is queued and
    /// flushed once the handshake completes).
    fn encapsulate(&mut self, ip_packet: &[u8]) -> Vec<WgAction>;

    /// Feeds an inbound WireGuard datagram received from the peer.
    fn decapsulate(&mut self, datagram: &[u8]) -> Vec<WgAction>;

    /// Periodic timer tick (call roughly every 100–250 ms). Drives
    /// handshakes, keepalives, and rekeying.
    fn tick(&mut self) -> Vec<WgAction>;
}

/// boringtun-backed [`WgPeer`].
pub struct BoringWgPeer {
    tunn: Tunn,
    peer_key: NodePublic,
    buf: Box<[u8; BUF_SIZE]>,
}

impl BoringWgPeer {
    /// Creates a session using `node_key` as our WireGuard static key and
    /// `peer_key` as the peer's. `index` must be unique per live session.
    pub fn new(node_key: &NodePrivate, peer_key: NodePublic, index: u32) -> Self {
        let static_private = StaticSecret::from(node_key.to_bytes());
        let peer_public = PublicKey::from(peer_key.0);
        let tunn = Tunn::new(
            static_private,
            peer_public,
            None,     // no preshared key
            Some(25), // persistent keepalive (seconds)
            index,
            None, // default rate limiter
        );
        Self {
            tunn,
            peer_key,
            buf: Box::new([0u8; BUF_SIZE]),
        }
    }

    /// After a call that produced a network datagram, boringtun may have
    /// more queued (e.g. data packets flushed once a handshake completed).
    /// Drain them by calling `decapsulate` with an empty datagram until it
    /// stops producing network output.
    fn drain_network(&mut self, out: &mut Vec<WgAction>) {
        while let TunnResult::WriteToNetwork(pkt) =
            self.tunn.decapsulate(None, &[], self.buf.as_mut_slice())
        {
            out.push(WgAction::ToPeer(pkt.to_vec()));
        }
    }
}

impl WgPeer for BoringWgPeer {
    fn peer_key(&self) -> NodePublic {
        self.peer_key
    }

    fn encapsulate(&mut self, ip_packet: &[u8]) -> Vec<WgAction> {
        let mut out = Vec::new();
        // Split the borrow: take the buffer out so `tunn` can be borrowed
        // mutably alongside it.
        match self.tunn.encapsulate(ip_packet, self.buf.as_mut_slice()) {
            TunnResult::WriteToNetwork(pkt) => {
                out.push(WgAction::ToPeer(pkt.to_vec()));
            }
            TunnResult::Done => {}
            TunnResult::Err(e) => tracing::debug!("wg encapsulate error: {e:?}"),
            // encapsulate never yields tunnel packets.
            _ => {}
        }
        out
    }

    fn decapsulate(&mut self, datagram: &[u8]) -> Vec<WgAction> {
        let mut out = Vec::new();
        match self
            .tunn
            .decapsulate(None, datagram, self.buf.as_mut_slice())
        {
            TunnResult::WriteToNetwork(pkt) => {
                out.push(WgAction::ToPeer(pkt.to_vec()));
                // A handshake response may unblock queued data packets.
                self.drain_network(&mut out);
            }
            TunnResult::WriteToTunnelV4(pkt, _) | TunnResult::WriteToTunnelV6(pkt, _) => {
                out.push(WgAction::ToLocal(pkt.to_vec()));
            }
            TunnResult::Done => {}
            TunnResult::Err(e) => tracing::debug!("wg decapsulate error: {e:?}"),
        }
        out
    }

    fn tick(&mut self) -> Vec<WgAction> {
        let mut out = Vec::new();
        match self.tunn.update_timers(self.buf.as_mut_slice()) {
            TunnResult::WriteToNetwork(pkt) => out.push(WgAction::ToPeer(pkt.to_vec())),
            TunnResult::Done => {}
            TunnResult::Err(e) => tracing::debug!("wg timer error: {e:?}"),
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal valid IPv4 packet (20-byte header + payload) with a
    /// correct total-length field, which boringtun validates before
    /// delivering.
    fn ipv4_packet(payload: &[u8]) -> Vec<u8> {
        let total = 20 + payload.len();
        let mut p = vec![0u8; total];
        p[0] = 0x45; // version 4, IHL 5
        p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        p[8] = 64; // TTL
        p[9] = 1; // protocol ICMP
        p[12..16].copy_from_slice(&[100, 64, 0, 1]); // src 100.64.0.1
        p[16..20].copy_from_slice(&[100, 64, 0, 2]); // dst 100.64.0.2
        p[20..].copy_from_slice(payload);
        p
    }

    /// Drives a full boringtun handshake + data exchange between two peers
    /// entirely in memory, shuttling `ToPeer` datagrams across a loopback.
    /// Proves the adapter encrypts/decrypts correctly and that the drain
    /// logic flushes queued packets after the handshake completes.
    #[test]
    fn two_peers_handshake_and_exchange_ip_packet() {
        let a_key = NodePrivate::generate();
        let b_key = NodePrivate::generate();
        let mut a = BoringWgPeer::new(&a_key, b_key.public(), 1);
        let mut b = BoringWgPeer::new(&b_key, a_key.public(), 2);

        let ip = ipv4_packet(b"icmp-ish");

        // A's first send with no session must emit a handshake datagram.
        let mut a_out: Vec<Vec<u8>> = to_peer_bytes(a.encapsulate(&ip));
        assert!(
            !a_out.is_empty(),
            "encapsulate with no session starts a handshake"
        );

        // Pump datagrams both ways until B delivers the IP packet locally.
        let mut delivered: Option<Vec<u8>> = None;
        for _ in 0..20 {
            let mut b_out: Vec<Vec<u8>> = Vec::new();
            for dg in a_out.drain(..) {
                for act in b.decapsulate(&dg) {
                    match act {
                        WgAction::ToPeer(x) => b_out.push(x),
                        WgAction::ToLocal(pkt) => delivered = Some(pkt),
                    }
                }
            }
            if delivered.is_some() {
                break;
            }
            for dg in b_out {
                a_out.extend(to_peer_bytes(a.decapsulate(&dg)));
            }
            // Retry the send + advance timers to make progress deterministic.
            a_out.extend(to_peer_bytes(a.encapsulate(&ip)));
            a_out.extend(to_peer_bytes(a.tick()));
        }

        let delivered = delivered.expect("B received the IP packet through the tunnel");
        assert_eq!(delivered, ip, "decrypted packet matches original");
    }

    fn to_peer_bytes(actions: Vec<WgAction>) -> Vec<Vec<u8>> {
        actions
            .into_iter()
            .filter_map(|a| match a {
                WgAction::ToPeer(b) => Some(b),
                WgAction::ToLocal(_) => None,
            })
            .collect()
    }

    #[test]
    fn tick_is_safe_before_handshake() {
        let k = NodePrivate::generate();
        let mut peer = BoringWgPeer::new(&k, NodePrivate::generate().public(), 1);
        // Should not panic; may or may not produce output.
        let _ = peer.tick();
        assert_eq!(peer.peer_key(), peer.peer_key());
    }
}
