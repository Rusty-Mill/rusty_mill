//! IP address facts shared by outbound-address guards.
//!
//! This is deliberately not a global-routability or connection-permission API.
//! Callers choose which classes to reject and whether to interpret embedded IPv4.
//! [`AddressClass::Other`] includes unlisted special-use/documentation addresses;
//! it does not mean public or safe. There is no DNS, URL, transport, or opt-in policy.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use core::net::{IpAddr, Ipv4Addr};

/// An address fact, not an allow/deny decision or exhaustive IANA classification.
///
/// Overlapping classes use the more specific variant: unspecified before
/// `ThisNetwork`, and limited broadcast before `Reserved`. Embedded IPv4 is
/// returned as data so callers can retain distinct mapped/NAT64 policies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressClass {
    /// `0.0.0.0` or `::`.
    Unspecified,
    /// IPv4 `127.0.0.0/8` or IPv6 `::1`.
    Loopback,
    /// RFC1918 IPv4: `10/8`, `172.16/12`, or `192.168/16`.
    Private,
    /// IPv4 `169.254/16` or IPv6 `fe80::/10`.
    LinkLocal,
    /// IPv4 `100.64.0.0/10` shared address space.
    Shared,
    /// IPv4 limited broadcast `255.255.255.255`.
    Broadcast,
    /// IPv4 `224/4` or IPv6 `ff00::/8`.
    Multicast,
    /// IPv4 `0/8`, except the unspecified address.
    ThisNetwork,
    /// The whole IPv4 `192.0.0.0/24`, without registry-specific exceptions.
    ProtocolAssignment,
    /// IPv4 `198.18.0.0/15` benchmarking range.
    Benchmarking,
    /// IPv4 `240/4`, except limited broadcast.
    Reserved,
    /// IPv6 `fc00::/7` unique-local range.
    UniqueLocal,
    /// Deprecated IPv6 `fec0::/10` site-local range.
    SiteLocal,
    /// IPv4 mapped in IPv6 `::ffff:0:0/96` (not the compatible `::/96` form).
    Ipv4Mapped(Ipv4Addr),
    /// IPv4 embedded in the well-known NAT64 `64:ff9b::/96` prefix only.
    Nat64(Ipv4Addr),
    /// None of the listed classes; makes no claim about reachability or safety.
    Other,
}

/// Classify an IP literal without normalizing away its embedded-address form.
///
/// Only well-known NAT64 `/96` is recognized; local-use NAT64 and deprecated
/// IPv4-compatible IPv6 retain native IPv6 classification. Call this again on an
/// embedded IPv4 address only if the caller's explicit policy requires it.
#[must_use]
pub fn classify(ip: IpAddr) -> AddressClass {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return AddressClass::Ipv4Mapped(v4);
            }
            let seg = v6.segments();
            if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [a, b] = seg[6].to_be_bytes();
                let [c, d] = seg[7].to_be_bytes();
                return AddressClass::Nat64(Ipv4Addr::new(a, b, c, d));
            }
            if v6.is_unspecified() {
                AddressClass::Unspecified
            } else if v6.is_loopback() {
                AddressClass::Loopback
            } else if v6.is_multicast() {
                AddressClass::Multicast
            } else if (seg[0] & 0xffc0) == 0xfe80 {
                AddressClass::LinkLocal
            } else if (seg[0] & 0xfe00) == 0xfc00 {
                AddressClass::UniqueLocal
            } else if (seg[0] & 0xffc0) == 0xfec0 {
                AddressClass::SiteLocal
            } else {
                AddressClass::Other
            }
        }
    }
}

fn classify_v4(v4: Ipv4Addr) -> AddressClass {
    let [a, b, c, ..] = v4.octets();
    if v4.is_unspecified() {
        AddressClass::Unspecified
    } else if v4.is_loopback() {
        AddressClass::Loopback
    } else if v4.is_private() {
        AddressClass::Private
    } else if v4.is_link_local() {
        AddressClass::LinkLocal
    } else if v4.is_broadcast() {
        AddressClass::Broadcast
    } else if v4.is_multicast() {
        AddressClass::Multicast
    } else if a == 0 {
        AddressClass::ThisNetwork
    } else if a == 100 && (64..128).contains(&b) {
        AddressClass::Shared
    } else if a == 192 && b == 0 && c == 0 {
        AddressClass::ProtocolAssignment
    } else if a == 198 && (b == 18 || b == 19) {
        AddressClass::Benchmarking
    } else if a >= 240 {
        AddressClass::Reserved
    } else {
        AddressClass::Other
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, AddressClass as Class};
    use core::net::Ipv4Addr;

    #[test]
    fn native_facts_distinguish_overlapping_classes() {
        for (ip, expected) in [
            ("0.0.0.0", Class::Unspecified),
            ("0.0.0.1", Class::ThisNetwork),
            ("127.0.0.1", Class::Loopback),
            ("10.0.0.1", Class::Private),
            ("169.254.169.254", Class::LinkLocal),
            ("100.64.0.0", Class::Shared),
            ("224.0.0.0", Class::Multicast),
            ("240.0.0.0", Class::Reserved),
            ("255.255.255.254", Class::Reserved),
            ("255.255.255.255", Class::Broadcast),
            ("192.0.0.9", Class::ProtocolAssignment),
            ("198.19.255.255", Class::Benchmarking),
            ("::", Class::Unspecified),
            ("::1", Class::Loopback),
            ("fc00::", Class::UniqueLocal),
            ("fe80::", Class::LinkLocal),
            ("fec0::", Class::SiteLocal),
            ("ff00::", Class::Multicast),
            ("203.0.113.5", Class::Other),
            ("2001:db8::", Class::Other),
        ] {
            assert_eq!(classify(ip.parse().unwrap()), expected, "{ip}");
        }
    }

    #[test]
    fn embedded_forms_are_facts_not_automatic_normalization() {
        let private = Ipv4Addr::new(10, 0, 0, 1);
        for (ip, expected) in [
            ("::ffff:10.0.0.1", Class::Ipv4Mapped(private)),
            ("64:ff9b::a00:1", Class::Nat64(private)),
            ("::10.0.0.1", Class::Other),
            ("64:ff9b:1::a00:1", Class::Other),
            ("::fffe:ffff:ffff", Class::Other),
            ("::1:0:0:0", Class::Other),
            ("64:ff9a:ffff:ffff:ffff:ffff:ffff:ffff", Class::Other),
            ("64:ff9b::1:0:0", Class::Other),
        ] {
            assert_eq!(classify(ip.parse().unwrap()), expected, "{ip}");
        }
    }

    #[test]
    fn native_prefix_boundaries_have_the_documented_facts() {
        for (before, first, last, after, class) in [
            (
                "9.255.255.255",
                "10.0.0.0",
                "10.255.255.255",
                "11.0.0.0",
                Class::Private,
            ),
            (
                "172.15.255.255",
                "172.16.0.0",
                "172.31.255.255",
                "172.32.0.0",
                Class::Private,
            ),
            (
                "192.167.255.255",
                "192.168.0.0",
                "192.168.255.255",
                "192.169.0.0",
                Class::Private,
            ),
            (
                "126.255.255.255",
                "127.0.0.0",
                "127.255.255.255",
                "128.0.0.0",
                Class::Loopback,
            ),
            (
                "169.253.255.255",
                "169.254.0.0",
                "169.254.255.255",
                "169.255.0.0",
                Class::LinkLocal,
            ),
            (
                "100.63.255.255",
                "100.64.0.0",
                "100.127.255.255",
                "100.128.0.0",
                Class::Shared,
            ),
            (
                "191.255.255.255",
                "192.0.0.0",
                "192.0.0.255",
                "192.0.1.0",
                Class::ProtocolAssignment,
            ),
            (
                "198.17.255.255",
                "198.18.0.0",
                "198.19.255.255",
                "198.20.0.0",
                Class::Benchmarking,
            ),
            (
                "fbff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
                "fc00::",
                "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
                "fe00::",
                Class::UniqueLocal,
            ),
        ] {
            for ip in [before, after] {
                assert_eq!(classify(ip.parse().unwrap()), Class::Other, "{ip}");
            }
            for ip in [first, last] {
                assert_eq!(classify(ip.parse().unwrap()), class, "{ip}");
            }
        }
        // Adjacent classified ranges must not hide an off-by-one transition.
        for (ip, expected) in [
            ("0.255.255.255", Class::ThisNetwork),
            ("1.0.0.0", Class::Other),
            ("223.255.255.255", Class::Other),
            ("239.255.255.255", Class::Multicast),
            ("240.0.0.0", Class::Reserved),
            ("fe7f:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Class::Other),
            ("fe80::", Class::LinkLocal),
            ("febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Class::LinkLocal),
            ("fec0::", Class::SiteLocal),
            ("feff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Class::SiteLocal),
            ("ff00::", Class::Multicast),
            ("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Class::Multicast),
        ] {
            assert_eq!(classify(ip.parse().unwrap()), expected, "{ip}");
        }
    }
}
