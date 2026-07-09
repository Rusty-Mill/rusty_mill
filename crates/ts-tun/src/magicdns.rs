//! MagicDNS, hosts-file stub (Phase 4).
//!
//! The design plan calls for a "hosts-style stub first". Rather than run a
//! resolver on 100.100.100.100:53 (a later refinement), we render the
//! netmap's peer names into a managed block in a hosts file. The block is
//! delimited by markers so it can be rewritten idempotently without touching
//! the user's own entries.
//!
//! A full MagicDNS resolver (its own UDP :53 responder, search domains,
//! split DNS) arrives with the daemon-hardening phase.

use std::net::Ipv4Addr;

pub const BEGIN_MARKER: &str = "# BEGIN tailscale-rs MagicDNS";
pub const END_MARKER: &str = "# END tailscale-rs MagicDNS";

/// One name→address mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEntry {
    pub ip: Ipv4Addr,
    /// Fully qualified name, e.g. `node1.tailnet.test` (no trailing dot).
    pub fqdn: String,
}

impl HostEntry {
    /// Builds an entry from a netmap DNS name (which carries a trailing dot)
    /// and an address. Returns `None` if the name is empty.
    pub fn from_dns_name(ip: Ipv4Addr, dns_name: &str) -> Option<Self> {
        let fqdn = dns_name.trim_end_matches('.').trim().to_string();
        if fqdn.is_empty() {
            return None;
        }
        Some(HostEntry { ip, fqdn })
    }

    /// The short hostname (first label).
    fn short(&self) -> &str {
        self.fqdn.split('.').next().unwrap_or(&self.fqdn)
    }
}

/// Renders the managed hosts block (including markers) for the given entries.
pub fn render_block(entries: &[HostEntry]) -> String {
    let mut out = String::new();
    out.push_str(BEGIN_MARKER);
    out.push('\n');
    for e in entries {
        let short = e.short();
        if short != e.fqdn {
            out.push_str(&format!("{}\t{} {}\n", e.ip, e.fqdn, short));
        } else {
            out.push_str(&format!("{}\t{}\n", e.ip, e.fqdn));
        }
    }
    out.push_str(END_MARKER);
    out.push('\n');
    out
}

/// Replaces (or appends) the managed block in an existing hosts file's
/// contents, preserving everything outside the markers.
pub fn merge_into(existing: &str, entries: &[HostEntry]) -> String {
    let block = render_block(entries);
    let (before, after) = match (existing.find(BEGIN_MARKER), existing.find(END_MARKER)) {
        (Some(b), Some(e)) if e > b => {
            // Everything up to the begin marker, and everything after the
            // end-marker line.
            let end_line_end = existing[e..]
                .find('\n')
                .map(|off| e + off + 1)
                .unwrap_or(existing.len());
            (&existing[..b], &existing[end_line_end..])
        }
        _ => {
            // No managed block yet: append.
            let sep = if existing.is_empty() || existing.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            return format!("{existing}{sep}{block}");
        }
    };
    format!("{before}{block}{after}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<HostEntry> {
        vec![
            HostEntry::from_dns_name(Ipv4Addr::new(100, 64, 0, 1), "node1.tailnet.test.").unwrap(),
            HostEntry::from_dns_name(Ipv4Addr::new(100, 64, 0, 2), "node2.tailnet.test.").unwrap(),
        ]
    }

    #[test]
    fn from_dns_name_strips_trailing_dot() {
        let e = HostEntry::from_dns_name(Ipv4Addr::LOCALHOST, "a.b.c.").unwrap();
        assert_eq!(e.fqdn, "a.b.c");
        assert_eq!(e.short(), "a");
        assert!(HostEntry::from_dns_name(Ipv4Addr::LOCALHOST, ".").is_none());
        assert!(HostEntry::from_dns_name(Ipv4Addr::LOCALHOST, "").is_none());
    }

    #[test]
    fn render_has_markers_and_entries() {
        let block = render_block(&entries());
        assert!(block.starts_with(BEGIN_MARKER));
        assert!(block.trim_end().ends_with(END_MARKER));
        assert!(block.contains("100.64.0.1\tnode1.tailnet.test node1"));
        assert!(block.contains("100.64.0.2\tnode2.tailnet.test node2"));
    }

    #[test]
    fn merge_appends_when_absent() {
        let existing = "127.0.0.1 localhost\n";
        let merged = merge_into(existing, &entries());
        assert!(merged.starts_with("127.0.0.1 localhost\n"));
        assert!(merged.contains("node1.tailnet.test"));
    }

    #[test]
    fn merge_replaces_existing_block_idempotently() {
        let existing = "127.0.0.1 localhost\n";
        let once = merge_into(existing, &entries());
        // Re-merging with different entries replaces the block, keeps prefix.
        let new_entries = vec![
            HostEntry::from_dns_name(Ipv4Addr::new(100, 64, 0, 9), "node9.tailnet.test.").unwrap(),
        ];
        let twice = merge_into(&once, &new_entries);
        assert!(twice.starts_with("127.0.0.1 localhost\n"));
        assert!(twice.contains("node9.tailnet.test"));
        assert!(!twice.contains("node1.tailnet.test"), "old entries removed");
        // Only one managed block.
        assert_eq!(twice.matches(BEGIN_MARKER).count(), 1);
    }

    #[test]
    fn merge_is_stable_on_repeat() {
        let existing = "127.0.0.1 localhost\n";
        let a = merge_into(existing, &entries());
        let b = merge_into(&a, &entries());
        assert_eq!(a, b, "re-applying the same entries is a no-op");
    }
}
