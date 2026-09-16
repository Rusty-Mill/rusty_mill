//! Minimal STUN client (RFC 5389 binding requests), enough to learn a
//! node's server-reflexive (public) UDP endpoint for NAT traversal.
//!
//! Mirrors Go `net/stun`: we send a binding request and parse
//! `XOR-MAPPED-ADDRESS` from the success response. Parsing is panic-free on
//! arbitrary input (a `cargo-fuzz` target lands with the crate).

#![forbid(unsafe_code)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use rand_core::{OsRng, RngCore};

const MAGIC_COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];
const BINDING_REQUEST: [u8; 2] = [0x00, 0x01];
const BINDING_SUCCESS: [u8; 2] = [0x01, 0x01];
const HEADER_LEN: usize = 20;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const ATTR_XOR_MAPPED_ADDRESS_ALT: u16 = 0x8020;

/// A 96-bit STUN transaction ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxId(pub [u8; 12]);

impl TxId {
    /// Generates a random transaction ID from the OS RNG.
    pub fn random() -> Self {
        let mut id = [0u8; 12];
        OsRng.fill_bytes(&mut id);
        Self(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("not a STUN message")]
    NotStun,
    #[error("not a binding success response")]
    NotSuccess,
    #[error("transaction ID mismatch")]
    TxIdMismatch,
    #[error("malformed STUN attributes")]
    Malformed,
    #[error("no mapped-address attribute in response")]
    NoAddress,
}

/// Builds a binding request with the given transaction ID.
///
/// Minimal form: header only, no SOFTWARE or FINGERPRINT attributes (which
/// are optional; real STUN servers, including Headscale's embedded relay,
/// answer a bare request).
pub fn binding_request(tx: TxId) -> [u8; HEADER_LEN] {
    let mut b = [0u8; HEADER_LEN];
    b[0..2].copy_from_slice(&BINDING_REQUEST);
    // b[2..4] = attribute length = 0
    b[4..8].copy_from_slice(&MAGIC_COOKIE);
    b[8..20].copy_from_slice(&tx.0);
    b
}

/// Parses a binding success response, returning the reflexive address from
/// `XOR-MAPPED-ADDRESS`. Verifies the transaction ID against `expected`.
pub fn parse_response(buf: &[u8], expected: TxId) -> Result<SocketAddr, ParseError> {
    if buf.len() < HEADER_LEN {
        return Err(ParseError::NotStun);
    }
    if buf[4..8] != MAGIC_COOKIE {
        return Err(ParseError::NotStun);
    }
    if buf[0..2] != BINDING_SUCCESS {
        return Err(ParseError::NotSuccess);
    }
    if buf[8..20] != expected.0 {
        return Err(ParseError::TxIdMismatch);
    }

    let attrs_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    let attrs = buf.get(HEADER_LEN..).ok_or(ParseError::Malformed)?;
    let attrs = attrs.get(..attrs_len).ok_or(ParseError::Malformed)?;

    let mut offset = 0;
    while offset + 4 <= attrs.len() {
        let attr_type = u16::from_be_bytes([attrs[offset], attrs[offset + 1]]);
        let attr_len = u16::from_be_bytes([attrs[offset + 2], attrs[offset + 3]]) as usize;
        let value_start = offset + 4;
        let value_end = value_start + attr_len;
        if value_end > attrs.len() {
            return Err(ParseError::Malformed);
        }
        if attr_type == ATTR_XOR_MAPPED_ADDRESS || attr_type == ATTR_XOR_MAPPED_ADDRESS_ALT {
            return parse_xor_mapped_address(&attrs[value_start..value_end], expected);
        }
        // Attributes are padded to a 4-byte boundary.
        offset = value_end + ((4 - (attr_len % 4)) % 4);
    }
    Err(ParseError::NoAddress)
}

fn parse_xor_mapped_address(attr: &[u8], tx: TxId) -> Result<SocketAddr, ParseError> {
    // reserved(1) + family(1) + xport(2) + xaddr(4 or 16)
    if attr.len() < 4 {
        return Err(ParseError::Malformed);
    }
    let family = attr[1];
    let xport = u16::from_be_bytes([attr[2], attr[3]]);
    let port = xport ^ u16::from_be_bytes([MAGIC_COOKIE[0], MAGIC_COOKIE[1]]);
    let xaddr = &attr[4..];

    // XOR key = magic cookie followed by the transaction ID.
    let mut key = [0u8; 16];
    key[0..4].copy_from_slice(&MAGIC_COOKIE);
    key[4..16].copy_from_slice(&tx.0);

    match family {
        0x01 => {
            if xaddr.len() < 4 {
                return Err(ParseError::Malformed);
            }
            let mut octets = [0u8; 4];
            for i in 0..4 {
                octets[i] = xaddr[i] ^ key[i];
            }
            Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(octets)), port))
        }
        0x02 => {
            if xaddr.len() < 16 {
                return Err(ParseError::Malformed);
            }
            let mut octets = [0u8; 16];
            for i in 0..16 {
                octets[i] = xaddr[i] ^ key[i];
            }
            Ok(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port))
        }
        _ => Err(ParseError::Malformed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_layout() {
        let tx = TxId([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        let req = binding_request(tx);
        assert_eq!(&req[0..2], &[0x00, 0x01]);
        assert_eq!(&req[2..4], &[0x00, 0x00]);
        assert_eq!(&req[4..8], &MAGIC_COOKIE);
        assert_eq!(&req[8..20], &tx.0);
    }

    /// Round-trip: build a response with a known XOR-mapped address (the way
    /// Go's `stun.Response` does) and parse it back.
    #[test]
    fn parse_xor_mapped_ipv4() {
        let tx = TxId([0xaa; 12]);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), 51820);
        let resp = build_success_response(tx, addr);
        assert_eq!(parse_response(&resp, tx).unwrap(), addr);
    }

    #[test]
    fn wrong_txid_rejected() {
        let tx = TxId([0xaa; 12]);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1);
        let resp = build_success_response(tx, addr);
        assert_eq!(
            parse_response(&resp, TxId([0xbb; 12])),
            Err(ParseError::TxIdMismatch)
        );
    }

    #[test]
    fn parse_is_panic_free_on_garbage() {
        let tx = TxId([0; 12]);
        for g in [
            &b""[..],
            &[0u8; 8],
            &[0u8; 20],
            &[0x01, 0x01, 0xff, 0xff, 0x21, 0x12, 0xa4, 0x42],
            &[
                0x01, 0x01, 0x00, 0x08, 0x21, 0x12, 0xa4, 0x42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0x00, 0x20, 0xff, 0xff,
            ],
        ] {
            let _ = parse_response(g, tx);
        }
    }

    /// Helper mirroring Go `stun.Response`, for the round-trip tests.
    fn build_success_response(tx: TxId, addr: SocketAddr) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&BINDING_SUCCESS);
        let (fam, addr_bytes): (u8, Vec<u8>) = match addr.ip() {
            IpAddr::V4(v4) => (0x01, v4.octets().to_vec()),
            IpAddr::V6(v6) => (0x02, v6.octets().to_vec()),
        };
        let attr_len = 4 + addr_bytes.len();
        b.extend_from_slice(&(attr_len as u16 + 4).to_be_bytes()); // attrs length
        b.extend_from_slice(&MAGIC_COOKIE);
        b.extend_from_slice(&tx.0);
        // XOR-MAPPED-ADDRESS attribute.
        b.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        b.extend_from_slice(&(attr_len as u16).to_be_bytes());
        b.push(0); // reserved
        b.push(fam);
        let xport = addr.port() ^ u16::from_be_bytes([MAGIC_COOKIE[0], MAGIC_COOKIE[1]]);
        b.extend_from_slice(&xport.to_be_bytes());
        let mut key = [0u8; 16];
        key[0..4].copy_from_slice(&MAGIC_COOKIE);
        key[4..16].copy_from_slice(&tx.0);
        for (i, byte) in addr_bytes.iter().enumerate() {
            b.push(byte ^ key[i]);
        }
        b
    }
}
