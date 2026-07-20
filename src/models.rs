//! Port of `src/models` — protocol-wide constants.
//!
//! The Go package also resolves the default relay hostnames at init time
//! (optionally through a hardcoded list of public DNS servers); that logic
//! lives with the caller here so library users aren't forced into DNS lookups.

/// Maximum packet size used when piping raw data through the relay.
pub const TCP_BUFFER_SIZE: usize = 1024 * 64;

pub const DEFAULT_RELAY: &str = "croc.schollz.com";
pub const DEFAULT_RELAY6: &str = "croc6.schollz.com";
pub const DEFAULT_PORT: &str = "9009";
pub const DEFAULT_PASSPHRASE: &str = "pass123";

use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

/// Public DNS resolvers queried directly when `--internal-dns` is set, so the
/// relay hostname resolves even when the local resolver is broken or
/// censored. Mirrors croc's `publicDNS` list (the IPv4 entries; the IPv6
/// resolvers there need working IPv6 egress and are skipped here to avoid
/// slow, doomed queries on IPv4-only hosts).
pub const PUBLIC_DNS: &[&str] = &[
    "1.1.1.1",         // Cloudflare
    "1.0.0.1",         // Cloudflare
    "8.8.8.8",         // Google
    "8.8.4.4",         // Google
    "9.9.9.9",         // Quad9
    "149.112.112.112", // Quad9
    "208.67.222.222",  // Cisco OpenDNS
    "208.67.220.220",  // Cisco OpenDNS
];

const DNS_QTYPE_A: u16 = 1;
const DNS_QTYPE_AAAA: u16 = 28;

/// Encode a DNS query for `host` of the given record type. Standard recursive
/// A/AAAA query, `id` in the header.
fn build_dns_query(id: u16, host: &str, qtype: u16) -> Vec<u8> {
    let mut q = Vec::with_capacity(host.len() + 18);
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&0x0100u16.to_be_bytes()); // flags: recursion desired
    q.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    q.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // AN/NS/AR counts = 0
    for label in host.split('.').filter(|l| !l.is_empty()) {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0); // root label
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&1u16.to_be_bytes()); // QCLASS = IN
    q
}

/// Advance past a DNS name at `pos`, honoring compression pointers. Returns
/// the offset just after the name (pointers count as 2 bytes and don't
/// require following, since we only need to skip).
fn skip_dns_name(msg: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = *msg.get(pos)?;
        if len & 0xc0 == 0xc0 {
            return Some(pos + 2); // compression pointer
        }
        if len == 0 {
            return Some(pos + 1); // root label ends the name
        }
        pos += 1 + len as usize;
    }
}

/// Parse the answer section of a DNS response, collecting IPs of `qtype`.
fn parse_dns_answers(msg: &[u8], qtype: u16) -> Vec<IpAddr> {
    let mut out = Vec::new();
    if msg.len() < 12 {
        return out;
    }
    let qd = u16::from_be_bytes([msg[4], msg[5]]);
    let an = u16::from_be_bytes([msg[6], msg[7]]);
    let mut pos = 12;
    // Skip the question section.
    for _ in 0..qd {
        pos = match skip_dns_name(msg, pos) {
            Some(p) => p + 4, // QTYPE + QCLASS
            None => return out,
        };
    }
    for _ in 0..an {
        pos = match skip_dns_name(msg, pos) {
            Some(p) => p,
            None => break,
        };
        if pos + 10 > msg.len() {
            break;
        }
        let rtype = u16::from_be_bytes([msg[pos], msg[pos + 1]]);
        let rdlen = u16::from_be_bytes([msg[pos + 8], msg[pos + 9]]) as usize;
        pos += 10;
        if pos + rdlen > msg.len() {
            break;
        }
        if rtype == qtype {
            let rdata = &msg[pos..pos + rdlen];
            if qtype == DNS_QTYPE_A && rdlen == 4 {
                out.push(IpAddr::from([rdata[0], rdata[1], rdata[2], rdata[3]]));
            } else if qtype == DNS_QTYPE_AAAA && rdlen == 16 {
                let mut b = [0u8; 16];
                b.copy_from_slice(rdata);
                out.push(IpAddr::from(b));
            }
        }
        pos += rdlen;
    }
    out
}

/// Query a single DNS server (host, no port) for `host`'s records of `qtype`.
fn dns_query(server: &str, host: &str, qtype: u16) -> std::io::Result<Vec<IpAddr>> {
    // Accept either "1.1.1.1" (default DNS port assumed) or an explicit
    // "host:port" (used by tests with a mock resolver).
    let server_addr: SocketAddr = match server.parse() {
        Ok(a) => a,
        Err(_) => {
            let bare = server.trim_start_matches('[').trim_end_matches(']');
            format!("{bare}:53")
                .parse()
                .map_err(|_| std::io::Error::other("bad dns server address"))?
        }
    };
    let bind = if server_addr.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let sock = UdpSocket::bind(bind)?;
    sock.set_read_timeout(Some(Duration::from_millis(500)))?;
    // Derive a query id from the host so concurrent queries differ; the exact
    // value doesn't matter for a single request/response.
    let id = host
        .bytes()
        .fold(0x1234u16, |a, b| a.wrapping_add(b as u16));
    sock.send_to(&build_dns_query(id, host, qtype), server_addr)?;
    let mut buf = [0u8; 512];
    let (n, _) = sock.recv_from(&mut buf)?;
    Ok(parse_dns_answers(&buf[..n], qtype))
}

/// Resolve `host` to an IP, querying the given DNS `servers` in parallel and
/// taking the first answer. A records are preferred; AAAA is a fallback.
fn resolve_via_servers(host: &str, servers: &[&str]) -> Option<IpAddr> {
    for qtype in [DNS_QTYPE_A, DNS_QTYPE_AAAA] {
        let (tx, rx) = std::sync::mpsc::channel();
        for server in servers {
            let tx = tx.clone();
            let server = server.to_string();
            let host = host.to_string();
            std::thread::spawn(move || {
                let ips = dns_query(&server, &host, qtype).unwrap_or_default();
                let _ = tx.send(ips.into_iter().next());
            });
        }
        drop(tx);
        while let Ok(res) = rx.recv() {
            if let Some(ip) = res {
                return Some(ip);
            }
        }
    }
    None
}

/// Resolve a relay hostname to an IP. IP literals are returned as-is. When
/// `internal_dns` is set, the hardcoded public resolvers are queried directly
/// (croc's `--internal-dns`); otherwise the system resolver is used. Mirrors
/// `models.lookup`.
pub fn resolve_host(host: &str, internal_dns: bool) -> Option<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    if internal_dns {
        resolve_via_servers(host, PUBLIC_DNS)
    } else {
        // System resolver, matching croc's localLookupIP.
        (host, 0u16).to_socket_addrs().ok()?.next().map(|s| s.ip())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn ip_literal_passes_through() {
        assert_eq!(
            resolve_host("192.0.2.7", true),
            Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7)))
        );
        assert_eq!(
            resolve_host("192.0.2.7", false),
            Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7)))
        );
    }

    #[test]
    fn local_resolver_handles_localhost() {
        let ip = resolve_host("localhost", false);
        assert!(ip.map(|i| i.is_loopback()).unwrap_or(false), "got {ip:?}");
    }

    #[test]
    fn query_round_trips_through_a_mock_dns_server() {
        // Mock resolver: echo the question, append one A record 203.0.113.9.
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            // Answer A queries in a loop; ignore AAAA (respond with no records)
            // so the resolver's A path wins.
            while let Ok((n, src)) = server.recv_from(&mut buf) {
                let query = buf[..n].to_vec();
                let qtype = if n >= 2 {
                    u16::from_be_bytes([query[n - 4], query[n - 3]])
                } else {
                    0
                };
                let mut resp = Vec::new();
                resp.extend_from_slice(&query[0..2]); // id
                resp.extend_from_slice(&0x8180u16.to_be_bytes()); // response, RD+RA
                resp.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
                let ancount: u16 = if qtype == 1 { 1 } else { 0 };
                resp.extend_from_slice(&ancount.to_be_bytes());
                resp.extend_from_slice(&[0, 0, 0, 0]); // NS/AR
                resp.extend_from_slice(&query[12..]); // echo question
                if ancount == 1 {
                    resp.extend_from_slice(&0xc00cu16.to_be_bytes()); // name ptr → 12
                    resp.extend_from_slice(&1u16.to_be_bytes()); // TYPE A
                    resp.extend_from_slice(&1u16.to_be_bytes()); // CLASS IN
                    resp.extend_from_slice(&60u32.to_be_bytes()); // TTL
                    resp.extend_from_slice(&4u16.to_be_bytes()); // RDLENGTH
                    resp.extend_from_slice(&[203, 0, 113, 9]); // RDATA
                }
                if server.send_to(&resp, src).is_err() {
                    break;
                }
            }
        });
        let host = format!("127.0.0.1:{}", addr.port());
        let ips = dns_query(&host, "relay.example.com", DNS_QTYPE_A).unwrap();
        assert_eq!(ips, vec![IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))]);

        let resolved = resolve_via_servers("relay.example.com", &[host.as_str()]);
        assert_eq!(resolved, Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))));
    }
}
