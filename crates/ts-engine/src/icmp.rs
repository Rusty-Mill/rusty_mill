//! Minimal userspace IPv4 + ICMP echo, panic-free on arbitrary input.
//!
//! Phase 3 has no TUN device (that is Phase 4), so the engine answers and
//! originates ICMP echoes itself over the WireGuard tunnel. This is just
//! enough IP/ICMP to prove relayed connectivity — not a network stack.

use std::net::Ipv4Addr;

const IPV4_MIN_HEADER: usize = 20;
pub const PROTO_ICMP: u8 = 1;
pub const ICMP_ECHO_REQUEST: u8 = 8;
pub const ICMP_ECHO_REPLY: u8 = 0;

/// A parsed view of an IPv4 packet's fixed header fields.
#[derive(Debug, Clone, Copy)]
pub struct Ipv4View {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub protocol: u8,
    /// Offset where the L4 payload (e.g. ICMP) begins.
    pub payload_offset: usize,
}

/// Parses the IPv4 header, returning `None` on anything malformed.
pub fn parse_ipv4(pkt: &[u8]) -> Option<Ipv4View> {
    if pkt.len() < IPV4_MIN_HEADER {
        return None;
    }
    if pkt[0] >> 4 != 4 {
        return None;
    }
    let ihl = (pkt[0] & 0x0f) as usize * 4;
    if ihl < IPV4_MIN_HEADER || pkt.len() < ihl {
        return None;
    }
    let src = Ipv4Addr::new(pkt[12], pkt[13], pkt[14], pkt[15]);
    let dst = Ipv4Addr::new(pkt[16], pkt[17], pkt[18], pkt[19]);
    Some(Ipv4View {
        src,
        dst,
        protocol: pkt[9],
        payload_offset: ihl,
    })
}

/// A parsed ICMP echo (request or reply).
#[derive(Debug, Clone)]
pub struct EchoView {
    pub is_reply: bool,
    pub id: u16,
    pub seq: u16,
    pub data: Vec<u8>,
}

/// Parses an ICMP echo message from an L4 payload, `None` if not an echo.
pub fn parse_echo(icmp: &[u8]) -> Option<EchoView> {
    if icmp.len() < 8 {
        return None;
    }
    let is_reply = match icmp[0] {
        ICMP_ECHO_REPLY => true,
        ICMP_ECHO_REQUEST => false,
        _ => return None,
    };
    Some(EchoView {
        is_reply,
        id: u16::from_be_bytes([icmp[4], icmp[5]]),
        seq: u16::from_be_bytes([icmp[6], icmp[7]]),
        data: icmp[8..].to_vec(),
    })
}

/// Builds a full IPv4+ICMP echo *request* packet.
pub fn build_echo_request(src: Ipv4Addr, dst: Ipv4Addr, id: u16, seq: u16, data: &[u8]) -> Vec<u8> {
    build_echo(src, dst, ICMP_ECHO_REQUEST, id, seq, data)
}

/// Given a received echo *request* IPv4 packet, builds the matching echo
/// *reply* (swaps addresses, flips the type). `None` if the input is not an
/// ICMP echo request.
pub fn build_echo_reply(request: &[u8]) -> Option<Vec<u8>> {
    let ip = parse_ipv4(request)?;
    if ip.protocol != PROTO_ICMP {
        return None;
    }
    let echo = parse_echo(request.get(ip.payload_offset..)?)?;
    if echo.is_reply {
        return None;
    }
    Some(build_echo(
        ip.dst, // reply src = request dst
        ip.src, // reply dst = request src
        ICMP_ECHO_REPLY,
        echo.id,
        echo.seq,
        &echo.data,
    ))
}

fn build_echo(
    src: Ipv4Addr,
    dst: Ipv4Addr,
    icmp_type: u8,
    id: u16,
    seq: u16,
    data: &[u8],
) -> Vec<u8> {
    let icmp_len = 8 + data.len();
    let total = IPV4_MIN_HEADER + icmp_len;
    let mut p = vec![0u8; total];

    // IPv4 header.
    p[0] = 0x45; // version 4, IHL 5
    p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    p[8] = 64; // TTL
    p[9] = PROTO_ICMP;
    p[12..16].copy_from_slice(&src.octets());
    p[16..20].copy_from_slice(&dst.octets());
    let ip_csum = checksum(&p[..IPV4_MIN_HEADER]);
    p[10..12].copy_from_slice(&ip_csum.to_be_bytes());

    // ICMP message.
    let icmp = &mut p[IPV4_MIN_HEADER..];
    icmp[0] = icmp_type;
    icmp[1] = 0; // code
    icmp[4..6].copy_from_slice(&id.to_be_bytes());
    icmp[6..8].copy_from_slice(&seq.to_be_bytes());
    icmp[8..].copy_from_slice(data);
    let icmp_csum = checksum(icmp);
    icmp[2..4].copy_from_slice(&icmp_csum.to_be_bytes());

    p
}

/// Standard 16-bit one's-complement Internet checksum (RFC 1071).
fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut chunks = data.chunks_exact(2);
    for c in &mut chunks {
        sum += u16::from_be_bytes([c[0], c[1]]) as u32;
    }
    if let [last] = chunks.remainder() {
        sum += (*last as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_reply_round_trip() {
        let a = Ipv4Addr::new(100, 64, 0, 1);
        let b = Ipv4Addr::new(100, 64, 0, 2);
        let req = build_echo_request(a, b, 0x1234, 7, b"ping-data");

        let ip = parse_ipv4(&req).unwrap();
        assert_eq!(ip.src, a);
        assert_eq!(ip.dst, b);
        assert_eq!(ip.protocol, PROTO_ICMP);
        let echo = parse_echo(&req[ip.payload_offset..]).unwrap();
        assert!(!echo.is_reply);
        assert_eq!(echo.id, 0x1234);
        assert_eq!(echo.seq, 7);
        assert_eq!(echo.data, b"ping-data");

        let reply = build_echo_reply(&req).unwrap();
        let rip = parse_ipv4(&reply).unwrap();
        assert_eq!(rip.src, b, "reply src is request dst");
        assert_eq!(rip.dst, a, "reply dst is request src");
        let recho = parse_echo(&reply[rip.payload_offset..]).unwrap();
        assert!(recho.is_reply);
        assert_eq!(recho.id, 0x1234);
        assert_eq!(recho.seq, 7);
        assert_eq!(recho.data, b"ping-data");
    }

    #[test]
    fn checksums_are_valid() {
        // A correct Internet checksum makes the sum over the region zero.
        let req = build_echo_request(
            Ipv4Addr::new(1, 2, 3, 4),
            Ipv4Addr::new(5, 6, 7, 8),
            1,
            1,
            b"x",
        );
        assert_eq!(checksum(&req[..20]), 0, "IPv4 header verifies");
        assert_eq!(checksum(&req[20..]), 0, "ICMP message verifies");
    }

    #[test]
    fn parsers_reject_garbage_without_panicking() {
        for g in [
            &b""[..],
            &[0u8; 3],
            &[0x45; 8],
            &[0x60; 40], // IPv6 version nibble
            &[0x4f; 20], // IHL claims 60 bytes but only 20 present
        ] {
            let _ = parse_ipv4(g);
            let _ = build_echo_reply(g);
        }
        assert!(parse_echo(&[0u8; 4]).is_none());
        assert!(parse_echo(&[9u8; 8]).is_none(), "non-echo type rejected");
    }

    #[test]
    fn reply_of_reply_is_none() {
        let reply = build_echo_request(Ipv4Addr::LOCALHOST, Ipv4Addr::LOCALHOST, 1, 1, b"");
        let reply = build_echo_reply(&reply).unwrap();
        assert!(build_echo_reply(&reply).is_none(), "can't reply to a reply");
    }
}
