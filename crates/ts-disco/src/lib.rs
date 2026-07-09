//! Disco: the NAT-traversal message layer, mirroring Go `disco` (see
//! PROTOCOL.md).
//!
//! Disco messages ride either a direct UDP path or a DERP frame. On the wire:
//!
//! ```text
//! [6]  magic "TS💬"  (0x54 53 f0 9f 92 ac)
//! [32] sender disco public key (cleartext)
//! [24] nonce
//! [N]  NaCl box: seal(msgType || version || data)
//! ```
//!
//! The box is `crypto_box` (X25519 + XSalsa20-Poly1305) between the two disco
//! keys — the same primitive DERP's handshake uses. Only ping/pong/
//! call-me-maybe are modeled (the UDP-relay message types are out of scope).
//! All parsing is panic-free on arbitrary input.

#![forbid(unsafe_code)]

use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use crypto_box::{
    PublicKey, SalsaBox, SecretKey,
    aead::{Aead, AeadCore, OsRng},
};
use ts_key::DiscoPrivate;
use ts_types::DiscoPublic;

/// 6-byte disco magic: `"TS"` + the speech-balloon emoji (U+1F4AC).
pub const MAGIC: &[u8; 6] = b"TS\xf0\x9f\x92\xac";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
/// Minimum wire size: magic + key + nonce + box tag.
const MIN_LEN: usize = 6 + KEY_LEN + NONCE_LEN + 16;

const VERSION: u8 = 0;
const TYPE_PING: u8 = 0x01;
const TYPE_PONG: u8 = 0x02;
const TYPE_CALL_ME_MAYBE: u8 = 0x03;
const TX_LEN: usize = 12;

/// A disco transaction ID (echoed from ping to pong).
pub type TxId = [u8; TX_LEN];

/// A decoded disco message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Probe of a candidate path. Carries the sender's node key.
    Ping {
        tx: TxId,
        node_key: ts_types::NodePublic,
    },
    /// Reply to a ping, reporting the source endpoint the ponger observed
    /// (so the pinger learns its own reflexive address).
    Pong { tx: TxId, src: SocketAddr },
    /// "Here are my candidate endpoints — please ping me." Sent over DERP to
    /// bootstrap hole punching.
    CallMeMaybe { endpoints: Vec<SocketAddr> },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiscoError {
    #[error("not a disco packet")]
    NotDisco,
    #[error("disco box failed to authenticate")]
    BadBox,
    #[error("unknown or malformed disco message")]
    Malformed,
}

/// Returns the sender's disco public key if `pkt` looks like a disco packet
/// (magic + at least a key). Used to route the packet to the right peer
/// before decryption.
pub fn source_key(pkt: &[u8]) -> Option<DiscoPublic> {
    if pkt.len() < 6 + KEY_LEN || &pkt[..6] != MAGIC {
        return None;
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&pkt[6..6 + KEY_LEN]);
    Some(DiscoPublic(key))
}

/// True if `pkt` starts with the disco magic (a cheap classifier for the
/// magicsock receive path: disco vs WireGuard).
pub fn is_disco(pkt: &[u8]) -> bool {
    pkt.len() >= 6 && &pkt[..6] == MAGIC
}

/// Seals `msg` into a disco packet from `sender` to `receiver_disco`.
pub fn seal(sender: &DiscoPrivate, receiver_disco: &DiscoPublic, msg: &Message) -> Vec<u8> {
    let plaintext = marshal(msg);
    let boxer = SalsaBox::new(
        &PublicKey::from(receiver_disco.0),
        &SecretKey::from(sender.to_bytes()),
    );
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let ciphertext = boxer
        .encrypt(&nonce, plaintext.as_slice())
        .expect("in-memory encryption is infallible");

    let mut pkt = Vec::with_capacity(6 + KEY_LEN + NONCE_LEN + ciphertext.len());
    pkt.extend_from_slice(MAGIC);
    pkt.extend_from_slice(&sender.public().0);
    pkt.extend_from_slice(nonce.as_slice());
    pkt.extend_from_slice(&ciphertext);
    pkt
}

/// Opens a disco packet addressed to us. Returns the sender's disco key and
/// the decoded message.
pub fn open(receiver: &DiscoPrivate, pkt: &[u8]) -> Result<(DiscoPublic, Message), DiscoError> {
    if pkt.len() < MIN_LEN || &pkt[..6] != MAGIC {
        return Err(DiscoError::NotDisco);
    }
    let mut sender_key = [0u8; KEY_LEN];
    sender_key.copy_from_slice(&pkt[6..6 + KEY_LEN]);
    let nonce = &pkt[6 + KEY_LEN..6 + KEY_LEN + NONCE_LEN];
    let ciphertext = &pkt[6 + KEY_LEN + NONCE_LEN..];

    let boxer = SalsaBox::new(
        &PublicKey::from(sender_key),
        &SecretKey::from(receiver.to_bytes()),
    );
    let plaintext = boxer
        .decrypt(nonce.into(), ciphertext)
        .map_err(|_| DiscoError::BadBox)?;
    let msg = unmarshal(&plaintext).ok_or(DiscoError::Malformed)?;
    Ok((DiscoPublic(sender_key), msg))
}

/// Serializes the inner (pre-encryption) message: `type || version || data`.
fn marshal(msg: &Message) -> Vec<u8> {
    let mut out = Vec::new();
    match msg {
        Message::Ping { tx, node_key } => {
            out.push(TYPE_PING);
            out.push(VERSION);
            out.extend_from_slice(tx);
            out.extend_from_slice(&node_key.0);
        }
        Message::Pong { tx, src } => {
            out.push(TYPE_PONG);
            out.push(VERSION);
            out.extend_from_slice(tx);
            out.extend_from_slice(&addr_to_16(src.ip()));
            out.extend_from_slice(&src.port().to_be_bytes());
        }
        Message::CallMeMaybe { endpoints } => {
            out.push(TYPE_CALL_ME_MAYBE);
            out.push(VERSION);
            for ep in endpoints {
                out.extend_from_slice(&addr_to_16(ep.ip()));
                out.extend_from_slice(&ep.port().to_be_bytes());
            }
        }
    }
    out
}

fn unmarshal(p: &[u8]) -> Option<Message> {
    if p.len() < 2 {
        return None;
    }
    let msg_type = p[0];
    let data = &p[2..]; // skip type + version
    match msg_type {
        TYPE_PING => {
            if data.len() < TX_LEN + KEY_LEN {
                return None;
            }
            let mut tx = [0u8; TX_LEN];
            tx.copy_from_slice(&data[..TX_LEN]);
            let mut nk = [0u8; KEY_LEN];
            nk.copy_from_slice(&data[TX_LEN..TX_LEN + KEY_LEN]);
            Some(Message::Ping {
                tx,
                node_key: ts_types::NodePublic(nk),
            })
        }
        TYPE_PONG => {
            if data.len() < TX_LEN + 16 + 2 {
                return None;
            }
            let mut tx = [0u8; TX_LEN];
            tx.copy_from_slice(&data[..TX_LEN]);
            let ip = addr_from_16(&data[TX_LEN..TX_LEN + 16])?;
            let port = u16::from_be_bytes([data[TX_LEN + 16], data[TX_LEN + 17]]);
            Some(Message::Pong {
                tx,
                src: SocketAddr::new(ip, port),
            })
        }
        TYPE_CALL_ME_MAYBE => {
            if !data.len().is_multiple_of(18) {
                return None;
            }
            let mut endpoints = Vec::with_capacity(data.len() / 18);
            for chunk in data.chunks_exact(18) {
                let ip = addr_from_16(&chunk[..16])?;
                let port = u16::from_be_bytes([chunk[16], chunk[17]]);
                endpoints.push(SocketAddr::new(ip, port));
            }
            Some(Message::CallMeMaybe { endpoints })
        }
        _ => None,
    }
}

/// Encodes an IP as 16 bytes (IPv4 as v4-mapped IPv6), matching the wire.
fn addr_to_16(ip: IpAddr) -> [u8; 16] {
    match ip {
        IpAddr::V4(v4) => v4.to_ipv6_mapped().octets(),
        IpAddr::V6(v6) => v6.octets(),
    }
}

/// Decodes a 16-byte wire address, unmapping v4-mapped IPv6 back to IPv4.
fn addr_from_16(b: &[u8]) -> Option<IpAddr> {
    if b.len() < 16 {
        return None;
    }
    let mut octets = [0u8; 16];
    octets.copy_from_slice(&b[..16]);
    let v6 = Ipv6Addr::from(octets);
    Some(match v6.to_ipv4_mapped() {
        Some(v4) => IpAddr::V4(v4),
        None => IpAddr::V6(v6),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn keys() -> (DiscoPrivate, DiscoPrivate) {
        (DiscoPrivate::generate(), DiscoPrivate::generate())
    }

    #[test]
    fn ping_round_trip() {
        let (a, b) = keys();
        let msg = Message::Ping {
            tx: [7u8; 12],
            node_key: ts_types::NodePublic([0x42; 32]),
        };
        let pkt = seal(&a, &b.public(), &msg);
        assert!(is_disco(&pkt));
        assert_eq!(source_key(&pkt), Some(a.public()));
        let (from, got) = open(&b, &pkt).unwrap();
        assert_eq!(from, a.public());
        assert_eq!(got, msg);
    }

    #[test]
    fn pong_round_trip_ipv4_and_ipv6() {
        let (a, b) = keys();
        for src in [
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)), 41641),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 3478),
        ] {
            let msg = Message::Pong { tx: [1u8; 12], src };
            let pkt = seal(&a, &b.public(), &msg);
            let (_, got) = open(&b, &pkt).unwrap();
            assert_eq!(got, Message::Pong { tx: [1u8; 12], src });
        }
    }

    #[test]
    fn call_me_maybe_round_trip() {
        let (a, b) = keys();
        let msg = Message::CallMeMaybe {
            endpoints: vec![
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)), 41641),
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)), 12345),
            ],
        };
        let pkt = seal(&a, &b.public(), &msg);
        let (_, got) = open(&b, &pkt).unwrap();
        assert_eq!(got, msg);
    }

    #[test]
    fn wrong_key_fails_to_open() {
        let (a, b) = keys();
        let c = DiscoPrivate::generate();
        let pkt = seal(&a, &b.public(), &Message::CallMeMaybe { endpoints: vec![] });
        // Opening with the wrong receiver key must fail, not panic.
        assert_eq!(open(&c, &pkt), Err(DiscoError::BadBox));
    }

    #[test]
    fn classifiers_and_open_are_panic_free_on_garbage() {
        let b = DiscoPrivate::generate();
        for g in [
            &b""[..],
            &[0u8; 5],
            MAGIC,
            &[MAGIC.as_slice(), &[0u8; 40]].concat(),
            &[MAGIC.as_slice(), &[0u8; 200]].concat(),
        ] {
            let _ = is_disco(g);
            let _ = source_key(g);
            let _ = open(&b, g);
        }
        // A WireGuard-looking packet is not disco.
        assert!(!is_disco(&[4u8, 0, 0, 0, 1]));
    }
}
