//! Hostname patterns.
//!
//! Gateway API allows a listener or route to name an exact hostname
//! (`api.example.com`), a single-label wildcard (`*.example.com`), or nothing
//! at all. The wildcard covers exactly one label — `*.example.com` matches
//! `api.example.com` but not `a.b.example.com`, and never the bare
//! `example.com`. Specificity is what breaks ties when several routes could
//! serve a request, so it is part of the type rather than recomputed later.

/// A hostname pattern from a listener or route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostnamePattern {
    /// Matches any hostname, including a missing `Host` header.
    Any,
    /// Matches one hostname exactly, case-insensitively.
    Exact(String),
    /// Matches exactly one label in place of the `*`.
    Wildcard(String),
}

impl HostnamePattern {
    /// Parse a pattern as it appears in configuration.
    pub fn parse(pattern: &str) -> Self {
        let pattern = pattern.trim().to_ascii_lowercase();
        match pattern.as_str() {
            "" | "*" => HostnamePattern::Any,
            _ => match pattern.strip_prefix("*.") {
                Some(suffix) => HostnamePattern::Wildcard(suffix.to_string()),
                None => HostnamePattern::Exact(pattern),
            },
        }
    }

    /// Whether `host` satisfies this pattern.
    ///
    /// `host` may carry a port (`example.com:8443`); it is stripped first,
    /// since the port is already decided by which socket accepted the
    /// connection.
    pub fn matches(&self, host: &str) -> bool {
        let host = strip_port(host).to_ascii_lowercase();
        match self {
            HostnamePattern::Any => true,
            HostnamePattern::Exact(expected) => host == *expected,
            HostnamePattern::Wildcard(suffix) => match host.strip_suffix(suffix) {
                // The remainder must be exactly one non-empty label plus its
                // dot: `api.` for `*.example.com`. Anything containing another
                // dot is a deeper subdomain and does not match.
                Some(prefix) => {
                    let Some(label) = prefix.strip_suffix('.') else {
                        return false;
                    };
                    !label.is_empty() && !label.contains('.')
                }
                None => false,
            },
        }
    }

    /// How specific this pattern is. Higher wins when breaking ties.
    pub fn specificity(&self) -> u8 {
        match self {
            HostnamePattern::Exact(_) => 2,
            HostnamePattern::Wildcard(_) => 1,
            HostnamePattern::Any => 0,
        }
    }
}

/// Strip a `:port` suffix, leaving IPv6 literals intact.
fn strip_port(host: &str) -> &str {
    // A bracketed IPv6 literal ends at `]`; anything after is the port.
    if let Some(end) = host.find(']') {
        return &host[..=end];
    }
    // An unbracketed host with more than one colon is a bare IPv6 literal, not
    // host:port — splitting it would corrupt the address.
    match host.split_once(':') {
        Some((head, tail)) if !tail.contains(':') => head,
        _ => host,
    }
}
