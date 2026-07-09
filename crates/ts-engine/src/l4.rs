//! Transport-layer (TCP/UDP) checksum recomputation.
//!
//! Packets the local kernel generates and routes into a TUN often carry an
//! *incomplete* transport checksum: with checksum offload the stack leaves
//! `CHECKSUM_PARTIAL` (only the pseudo-header sum in the field), expecting
//! "hardware" to finish it. A userspace TUN reads that partial packet, so
//! before relaying an outbound packet we recompute the TCP/UDP checksum in
//! full. ICMP is unaffected (the kernel always completes ICMP checksums),
//! which is why ping works over the tunnel but TCP would not without this.

const PROTO_TCP: u8 = 6;
const PROTO_UDP: u8 = 17;

/// If `pkt` is an IPv4 TCP or UDP packet, recomputes its transport checksum
/// in place. No-op for other packets or malformed input (panic-free).
pub fn fix_ipv4_transport_checksum(pkt: &mut [u8]) {
    if pkt.len() < 20 || pkt[0] >> 4 != 4 {
        return;
    }
    let ihl = (pkt[0] & 0x0f) as usize * 4;
    if ihl < 20 || pkt.len() < ihl {
        return;
    }
    let proto = pkt[9];
    let csum_field = match proto {
        PROTO_TCP => 16, // checksum at byte 16 of the TCP header
        PROTO_UDP => 6,  // checksum at byte 6 of the UDP header
        _ => return,
    };

    // L4 length from the IP total-length field, bounded by the buffer.
    let total_len = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
    let end = total_len.min(pkt.len());
    if end <= ihl || end - ihl < csum_field + 2 {
        return;
    }

    let mut src = [0u8; 4];
    let mut dst = [0u8; 4];
    src.copy_from_slice(&pkt[12..16]);
    dst.copy_from_slice(&pkt[16..20]);
    let l4_len = end - ihl;

    // Zero the checksum field, then sum pseudo-header + L4 segment.
    pkt[ihl + csum_field] = 0;
    pkt[ihl + csum_field + 1] = 0;

    let mut sum: u32 = 0;
    // Pseudo-header: src, dst, zero + proto, L4 length.
    for chunk in [&src[..], &dst[..]] {
        for pair in chunk.chunks_exact(2) {
            sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
        }
    }
    sum += proto as u32;
    sum += l4_len as u32;

    // L4 segment.
    let l4 = &pkt[ihl..end];
    let mut chunks = l4.chunks_exact(2);
    for pair in &mut chunks {
        sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
    }
    if let [last] = chunks.remainder() {
        sum += (*last as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let mut checksum = !(sum as u16);
    // UDP: a computed zero is transmitted as 0xFFFF (0 means "no checksum").
    if proto == PROTO_UDP && checksum == 0 {
        checksum = 0xFFFF;
    }
    pkt[ihl + csum_field..ihl + csum_field + 2].copy_from_slice(&checksum.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full ones-complement sum over pseudo-header + L4, including the
    /// checksum field, must be zero for a valid packet.
    fn l4_verifies(pkt: &[u8]) -> bool {
        let ihl = (pkt[0] & 0x0f) as usize * 4;
        let proto = pkt[9];
        let total = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
        let l4 = &pkt[ihl..total];
        let mut sum: u32 = 0;
        for pair in pkt[12..20].chunks_exact(2) {
            sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
        }
        sum += proto as u32 + (total - ihl) as u32;
        let mut chunks = l4.chunks_exact(2);
        for pair in &mut chunks {
            sum += u16::from_be_bytes([pair[0], pair[1]]) as u32;
        }
        if let [last] = chunks.remainder() {
            sum += (*last as u32) << 8;
        }
        while sum >> 16 != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        (sum as u16) == 0xffff || (sum as u16) == 0
    }

    fn ipv4_tcp(payload: &[u8]) -> Vec<u8> {
        let ihl = 20;
        let tcp_hdr = 20;
        let total = ihl + tcp_hdr + payload.len();
        let mut p = vec![0u8; total];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        p[8] = 64;
        p[9] = PROTO_TCP;
        p[12..16].copy_from_slice(&[100, 64, 0, 1]);
        p[16..20].copy_from_slice(&[100, 64, 0, 2]);
        // TCP header: src/dst ports, seq, ack, data-offset.
        p[20..22].copy_from_slice(&1234u16.to_be_bytes());
        p[22..24].copy_from_slice(&80u16.to_be_bytes());
        p[32] = (tcp_hdr as u8 / 4) << 4; // data offset
        p[40..].copy_from_slice(payload);
        p
    }

    #[test]
    fn fixes_tcp_checksum() {
        let mut pkt = ipv4_tcp(b"GET / HTTP/1.0\r\n\r\n");
        // Simulate CHECKSUM_PARTIAL: checksum field left non-final.
        pkt[20 + 16] = 0xab;
        pkt[20 + 16 + 1] = 0xcd;
        fix_ipv4_transport_checksum(&mut pkt);
        assert!(l4_verifies(&pkt), "TCP checksum must verify after fixup");
    }

    #[test]
    fn fixes_udp_checksum() {
        let ihl = 20;
        let total = ihl + 8 + 4;
        let mut p = vec![0u8; total];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        p[9] = PROTO_UDP;
        p[12..16].copy_from_slice(&[10, 0, 0, 1]);
        p[16..20].copy_from_slice(&[10, 0, 0, 2]);
        p[20..22].copy_from_slice(&5000u16.to_be_bytes());
        p[22..24].copy_from_slice(&5001u16.to_be_bytes());
        p[24..26].copy_from_slice(&12u16.to_be_bytes()); // UDP length
        p[28..].copy_from_slice(b"data");
        fix_ipv4_transport_checksum(&mut p);
        assert!(l4_verifies(&p), "UDP checksum must verify after fixup");
    }

    #[test]
    fn ignores_non_tcp_udp_and_garbage() {
        // ICMP (proto 1): untouched.
        let mut icmp = vec![
            0x45, 0, 0, 20, 0, 0, 0, 0, 64, 1, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2,
        ];
        let before = icmp.clone();
        fix_ipv4_transport_checksum(&mut icmp);
        assert_eq!(icmp, before);

        // Garbage must not panic.
        fix_ipv4_transport_checksum(&mut []);
        fix_ipv4_transport_checksum(&mut [0x45, 0, 0]);
        fix_ipv4_transport_checksum(&mut [0x60; 40]);
    }
}
