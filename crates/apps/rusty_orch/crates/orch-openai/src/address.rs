//! Which addresses the adapter may connect to: loopback and private only.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// True for loopback (`127.0.0.0/8`, `::1`), RFC 1918 (`10/8`, `172.16/12`,
/// `192.168/16`) and IPv6 unique-local (`fc00::/7`) addresses. An
/// IPv4-mapped IPv6 address is judged as the IPv4 address it carries.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_private_v4(v4),
            None => is_private_v6(v6),
        },
    }
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private()
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00
}

/// The resolved addresses when every one is private, else the first one
/// that is not. An empty answer is refused too, as `None`.
pub fn all_private(addrs: Vec<SocketAddr>) -> Result<Vec<SocketAddr>, Option<SocketAddr>> {
    if addrs.is_empty() {
        return Err(None);
    }
    match addrs.iter().find(|a| !is_private(a.ip())) {
        Some(public) => Err(Some(*public)),
        None => Ok(addrs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("test address")
    }

    #[test]
    fn private_table() {
        let cases = [
            ("127.0.0.1", true),
            ("127.255.255.254", true),
            ("10.0.0.1", true),
            ("10.255.255.255", true),
            ("172.16.0.1", true),
            ("172.31.255.255", true),
            ("172.15.255.255", false),
            ("172.32.0.0", false),
            ("192.168.1.10", true),
            ("192.169.0.1", false),
            ("169.254.1.1", false),
            ("100.64.0.1", false),
            ("0.0.0.0", false),
            ("255.255.255.255", false),
            ("8.8.8.8", false),
            ("::1", true),
            ("fc00::1", true),
            ("fdff:ffff::1", true),
            ("fe80::1", false),
            ("fec0::1", false),
            ("::", false),
            ("2001:db8::1", false),
            ("::ffff:10.1.2.3", true),
            ("::ffff:8.8.8.8", false),
        ];
        for (addr, expected) in cases {
            assert_eq!(is_private(ip(addr)), expected, "{addr}");
        }
    }

    fn sock(s: &str) -> SocketAddr {
        s.parse().expect("test socket address")
    }

    #[test]
    fn one_public_answer_refuses_the_lot() {
        let mixed = vec![sock("10.0.0.2:8080"), sock("203.0.113.9:8080")];
        assert_eq!(all_private(mixed), Err(Some(sock("203.0.113.9:8080"))));
        assert_eq!(all_private(vec![]), Err(None));
        let ok = vec![sock("127.0.0.1:1"), sock("[::1]:1")];
        assert_eq!(all_private(ok.clone()), Ok(ok));
    }
}
