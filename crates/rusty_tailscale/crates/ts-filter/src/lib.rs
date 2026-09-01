//! Netmap packet-filter (ACL) enforcement.
//!
//! The control server compiles the tailnet ACL into a flat allow-list of
//! [`FilterRule`]s and ships it in the netmap. This crate compiles those rules
//! into a fast matcher the engine consults for every inbound tunnelled packet:
//! a packet is delivered only if some rule permits `(src, dst, proto, port)`.
//!
//! Semantics follow Go's `wgengine/filter`:
//! * A rule matches when the source IP is in one of its `SrcIPs` **and** the
//!   destination matches one of its `DstPorts` **and** (if `IPProto` is
//!   non-empty) the protocol is listed.
//! * For port-bearing protocols (TCP/UDP/SCTP) the destination port must fall
//!   in the range. Port-less protocols (ICMP, …) match only a full-range
//!   (`0–65535`) destination, mirroring how the Go compiler treats "all
//!   ports" as "any protocol".
//!
//! The matcher is panic-free on any input (malformed CIDRs are dropped at
//! compile time; unknown protocols simply fail to match).

use std::net::IpAddr;

use ts_types::IpPrefix;
use ts_types::tailcfg::FilterRule;

/// IP protocol numbers we special-case (IANA).
mod proto {
    pub const TCP: u8 = 6;
    pub const UDP: u8 = 17;
    pub const SCTP: u8 = 132;
}

/// Returns true for protocols that carry L4 ports.
fn has_ports(protocol: u8) -> bool {
    matches!(protocol, proto::TCP | proto::UDP | proto::SCTP)
}

/// A source or destination network matcher: either "any" (`*`) or a CIDR.
#[derive(Debug, Clone, Copy)]
enum Net {
    Any,
    Prefix(IpPrefix),
}

impl Net {
    /// Parses a filter IP token: `"*"`, a CIDR (`100.64.0.0/10`), or a bare
    /// address (treated as a host route). Returns `None` on malformed input.
    fn parse(s: &str) -> Option<Net> {
        if s == "*" {
            return Some(Net::Any);
        }
        if s.contains('/') {
            return s.parse::<IpPrefix>().ok().map(Net::Prefix);
        }
        // Bare address → /32 or /128 host prefix.
        let addr: IpAddr = s.parse().ok()?;
        let bits = if addr.is_ipv4() { 32 } else { 128 };
        Some(Net::Prefix(IpPrefix { addr, bits }))
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match self {
            Net::Any => true,
            Net::Prefix(p) => prefix_contains(*p, ip),
        }
    }
}

/// True if `ip` lies within the CIDR `prefix` (same family, matching high bits).
fn prefix_contains(prefix: IpPrefix, ip: IpAddr) -> bool {
    match (prefix.addr, ip) {
        (IpAddr::V4(net), IpAddr::V4(ip)) => bits_match(&net.octets(), &ip.octets(), prefix.bits),
        (IpAddr::V6(net), IpAddr::V6(ip)) => bits_match(&net.octets(), &ip.octets(), prefix.bits),
        _ => false, // cross-family never matches
    }
}

/// Compares the first `bits` bits of two equal-length octet strings.
fn bits_match(a: &[u8], b: &[u8], bits: u8) -> bool {
    let full = (bits / 8) as usize;
    if a[..full] != b[..full] {
        return false;
    }
    let rem = bits % 8;
    if rem == 0 {
        return true;
    }
    let mask = 0xffu8 << (8 - rem);
    (a[full] & mask) == (b[full] & mask)
}

/// A compiled destination: network + inclusive port range.
#[derive(Debug, Clone, Copy)]
struct Dst {
    net: Net,
    first: u16,
    last: u16,
}

impl Dst {
    fn matches(&self, dst: IpAddr, protocol: u8, port: u16) -> bool {
        if !self.net.contains(dst) {
            return false;
        }
        if has_ports(protocol) {
            port >= self.first && port <= self.last
        } else {
            // Port-less protocol: only an all-ports destination permits it.
            self.first == 0 && self.last == u16::MAX
        }
    }
}

/// A compiled ACL rule.
#[derive(Debug, Clone)]
struct Rule {
    srcs: Vec<Net>,
    dsts: Vec<Dst>,
    /// Protocol numbers this rule is restricted to; empty means all.
    protos: Vec<u8>,
}

impl Rule {
    fn matches(&self, src: IpAddr, dst: IpAddr, protocol: u8, port: u16) -> bool {
        if !self.protos.is_empty() && !self.protos.contains(&protocol) {
            return false;
        }
        if !self.srcs.iter().any(|s| s.contains(src)) {
            return false;
        }
        self.dsts.iter().any(|d| d.matches(dst, protocol, port))
    }
}

/// A compiled packet filter: an allow-list of rules.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    rules: Vec<Rule>,
}

impl Filter {
    /// Compiles a flat list of [`FilterRule`]s. Malformed CIDRs and
    /// out-of-range protocol numbers are dropped (defensive: never panic on a
    /// hostile netmap).
    pub fn new(rules: &[FilterRule]) -> Self {
        let compiled = rules
            .iter()
            .map(|r| {
                let srcs = r.src_ips.iter().filter_map(|s| Net::parse(s)).collect();
                let dsts = r
                    .dst_ports
                    .iter()
                    .filter_map(|np| {
                        Net::parse(&np.ip).map(|net| Dst {
                            net,
                            first: np.ports.first,
                            last: np.ports.last,
                        })
                    })
                    .collect();
                let protos = r
                    .ip_proto
                    .iter()
                    .filter_map(|p| u8::try_from(*p).ok())
                    .collect();
                Rule { srcs, dsts, protos }
            })
            .collect();
        Filter { rules: compiled }
    }

    /// A filter that permits everything — the pre-netmap default and the
    /// effect of an empty ruleset would be *deny* all, so the engine uses this
    /// until the first real filter arrives to avoid a black-hole on startup.
    pub fn allow_all() -> Self {
        Filter {
            rules: vec![Rule {
                srcs: vec![Net::Any],
                dsts: vec![Dst {
                    net: Net::Any,
                    first: 0,
                    last: u16::MAX,
                }],
                protos: Vec::new(),
            }],
        }
    }

    /// Whether `src → dst` (protocol `protocol`, destination `port`) is
    /// permitted. `port` is ignored for port-less protocols.
    pub fn allows(&self, src: IpAddr, dst: IpAddr, protocol: u8, port: u16) -> bool {
        self.rules
            .iter()
            .any(|r| r.matches(src, dst, protocol, port))
    }

    /// Number of compiled rules (0 means deny-all).
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ts_types::tailcfg::{NetPortRange, PortRange};

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn rule(srcs: &[&str], dst: &str, first: u16, last: u16, protos: &[i32]) -> FilterRule {
        FilterRule {
            src_ips: srcs.iter().map(|s| s.to_string()).collect(),
            dst_ports: vec![NetPortRange {
                ip: dst.to_string(),
                ports: PortRange { first, last },
            }],
            ip_proto: protos.to_vec(),
        }
    }

    #[test]
    fn allow_all_permits_everything() {
        let f = Filter::allow_all();
        assert!(f.allows(ip("100.64.0.2"), ip("100.64.0.1"), proto::TCP, 22));
        assert!(f.allows(ip("100.64.0.2"), ip("100.64.0.1"), 1, 0)); // ICMP
        assert!(f.allows(ip("8.8.8.8"), ip("1.1.1.1"), proto::UDP, 53));
    }

    #[test]
    fn default_headscale_filter_allows_all_including_icmp() {
        // The exact shape Headscale sends with no ACL policy.
        let f = Filter::new(&[rule(&["*"], "*", 0, 65535, &[])]);
        assert!(f.allows(ip("100.64.0.2"), ip("100.64.0.1"), proto::TCP, 80));
        assert!(f.allows(ip("100.64.0.2"), ip("100.64.0.1"), 1, 0), "ICMP");
    }

    #[test]
    fn empty_ruleset_denies() {
        let f = Filter::new(&[]);
        assert_eq!(f.rule_count(), 0);
        assert!(!f.allows(ip("100.64.0.2"), ip("100.64.0.1"), proto::TCP, 22));
    }

    #[test]
    fn src_cidr_and_port_range_enforced() {
        // Allow only 100.64.0.0/24 → any, TCP ports 22–22.
        let f = Filter::new(&[rule(&["100.64.0.0/24"], "*", 22, 22, &[])]);
        assert!(f.allows(ip("100.64.0.9"), ip("100.64.0.1"), proto::TCP, 22));
        assert!(!f.allows(ip("100.64.0.9"), ip("100.64.0.1"), proto::TCP, 80));
        assert!(
            !f.allows(ip("100.65.0.9"), ip("100.64.0.1"), proto::TCP, 22),
            "src outside /24 denied"
        );
        // Port-less ICMP is NOT allowed by a narrow port range.
        assert!(
            !f.allows(ip("100.64.0.9"), ip("100.64.0.1"), 1, 0),
            "ICMP denied when only ports 22 allowed"
        );
    }

    #[test]
    fn ip_proto_restriction() {
        // Allow any src → any, but only UDP.
        let f = Filter::new(&[rule(&["*"], "*", 0, 65535, &[proto::UDP as i32])]);
        assert!(f.allows(ip("1.2.3.4"), ip("5.6.7.8"), proto::UDP, 53));
        assert!(!f.allows(ip("1.2.3.4"), ip("5.6.7.8"), proto::TCP, 53));
        // ICMP not in the proto list → denied even though ports are full.
        assert!(!f.allows(ip("1.2.3.4"), ip("5.6.7.8"), 1, 0));
    }

    #[test]
    fn dst_cidr_enforced() {
        let f = Filter::new(&[rule(&["*"], "100.64.0.0/10", 0, 65535, &[])]);
        assert!(f.allows(ip("1.2.3.4"), ip("100.64.0.1"), proto::TCP, 80));
        assert!(!f.allows(ip("1.2.3.4"), ip("9.9.9.9"), proto::TCP, 80));
    }

    #[test]
    fn malformed_rules_are_dropped_not_panicked() {
        let f = Filter::new(&[rule(&["not-an-ip", "*/999"], "garbage", 0, 65535, &[])]);
        // Both srcs dropped → rule can never match.
        assert!(!f.allows(ip("1.2.3.4"), ip("5.6.7.8"), proto::TCP, 1));
    }
}
