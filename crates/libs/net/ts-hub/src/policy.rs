//! Who may reach whom. The hub has no ACL language of its own: a rule names
//! mux principals, the same ids the mux server's allowlist uses.
//!
//! ```text
//! # who -> whom; `*` is anyone
//! ts:alice@example.com -> ts-node:mux.mesh
//! * -> ts-node:mux.mesh
//! ```
//!
//! A node's principal is `ts:LOGIN`, or `ts-node:NAME` when it registered
//! with a tagged key (see [`principal_of`]). No rule means no traffic: a
//! hub starts closed. A rule opens every port from the source to the
//! destination, one way. Each node's netmap lists only the peers it has a
//! rule with, in either direction, and its packet filter admits only the
//! sources that may reach it.

use ts_types::UserProfile;
use ts_types::tailcfg::{FilterRule, MapResponse, NetPortRange, Node, PortRange};

/// One side of a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// Any node.
    Any,
    /// The nodes whose principal is exactly this id.
    Principal(String),
}

impl Selector {
    fn matches(&self, principal: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Principal(id) => id == principal,
        }
    }

    fn parse(token: &str) -> Result<Self, String> {
        if token == "*" {
            return Ok(Self::Any);
        }
        let named = token
            .strip_prefix("ts:")
            .or_else(|| token.strip_prefix("ts-node:"))
            .is_some_and(|name| !name.is_empty());
        if named && !token.contains(char::is_whitespace) {
            return Ok(Self::Principal(token.to_string()));
        }
        Err(format!(
            "`{token}` is not `*`, `ts:LOGIN` or `ts-node:NAME`"
        ))
    }
}

/// `from` may reach `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub from: Selector,
    pub to: Selector,
}

/// A rule line that could not be read.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("policy line {line}: {reason}")]
pub struct PolicyError {
    pub line: usize,
    pub reason: String,
}

/// The hub's rules. The default allows nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    grants: Vec<Grant>,
}

impl Policy {
    /// Everyone may reach everyone.
    pub fn allow_all() -> Self {
        Self {
            grants: vec![Grant {
                from: Selector::Any,
                to: Selector::Any,
            }],
        }
    }

    /// Reads `FROM -> TO` lines; blank lines and `#` comments are skipped.
    pub fn parse(text: &str) -> Result<Self, PolicyError> {
        let mut grants = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let fail = |reason: String| PolicyError {
                line: i + 1,
                reason,
            };
            let (from, to) = line
                .split_once("->")
                .ok_or_else(|| fail("expected `FROM -> TO`".into()))?;
            grants.push(Grant {
                from: Selector::parse(from.trim()).map_err(&fail)?,
                to: Selector::parse(to.trim()).map_err(&fail)?,
            });
        }
        Ok(Self { grants })
    }

    fn allows(&self, from: &str, to: &str) -> bool {
        self.grants
            .iter()
            .any(|g| g.from.matches(from) && g.to.matches(to))
    }

    /// Narrows a full netmap (`node` plus every live peer) to what the
    /// policy lets `node` see, and sets its packet filter.
    pub fn apply(&self, map: &mut MapResponse) {
        let profiles = map.user_profiles.take().unwrap_or_default();
        let Some(me) = map.node.as_ref() else {
            map.user_profiles = Some(profiles);
            return;
        };
        let my_id = principal_of(me, &profiles);
        let my_ip = first_ip(me);
        let mut peers = map.peers.take().unwrap_or_default();
        let ids: Vec<Option<String>> = peers.iter().map(|p| principal_of(p, &profiles)).collect();

        // A peer without a principal can match nothing but `*`.
        let reach = |from: &Option<String>, to: &Option<String>| match (from, to) {
            (Some(f), Some(t)) => self.allows(f, t),
            _ => false,
        };
        let seen: Vec<bool> = ids
            .iter()
            .map(|id| reach(&my_id, id) || reach(id, &my_id))
            .collect();

        let mut rules = Vec::new();
        if let (Some(me_id), Some(ip)) = (&my_id, my_ip) {
            for grant in self.grants.iter().filter(|g| g.to.matches(me_id)) {
                let sources: Vec<String> = match &grant.from {
                    Selector::Any => vec!["*".to_string()],
                    from => peers
                        .iter()
                        .zip(&ids)
                        .zip(&seen)
                        .filter(|((_, id), seen)| {
                            **seen && id.as_deref().is_some_and(|i| from.matches(i))
                        })
                        .filter_map(|((p, _), _)| first_ip(p))
                        .collect(),
                };
                if !sources.is_empty() {
                    rules.push(FilterRule {
                        src_ips: sources,
                        dst_ports: vec![NetPortRange {
                            ip: ip.clone(),
                            ports: PortRange {
                                first: 0,
                                last: u16::MAX,
                            },
                        }],
                        ip_proto: Vec::new(),
                    });
                }
            }
        }

        let mut keep = seen.iter();
        peers.retain(|_| *keep.next().unwrap_or(&false));
        map.peers = Some(peers);
        map.user_profiles = Some(profiles);
        map.packet_filter = Some(rules);
    }
}

fn first_ip(node: &Node) -> Option<String> {
    node.addresses.first().map(|a| a.addr.to_string())
}

/// The principal a node has, by the rule the mux server uses: a tagged node
/// is `ts-node:NAME` (its name without the trailing dot), any other its
/// user's `ts:LOGIN`. `None` if the node has neither.
pub fn principal_of(node: &Node, profiles: &[UserProfile]) -> Option<String> {
    if !node.tags.is_empty() {
        let name = node.name.trim_end_matches('.');
        return (!name.is_empty()).then(|| format!("ts-node:{name}"));
    }
    profiles
        .iter()
        .find(|p| p.id == node.user)
        .map(|p| format!("ts:{}", p.login_name))
        .filter(|id| id.len() > 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_read_one_per_line_with_comments() {
        let p = Policy::parse(
            "# who -> whom\nts:alice@x.io -> ts-node:mux.mesh  # her laptop\n\n* -> ts-node:b.mesh\n",
        )
        .unwrap();
        assert_eq!(p.grants.len(), 2);
        assert_eq!(p.grants[1].from, Selector::Any);
    }

    #[test]
    fn a_bad_rule_names_its_line() {
        for (text, line) in [
            ("ts:a -> ts:b\nnope\n", 2),
            ("ts: -> ts:b", 1),
            ("root -> ts:b", 1),
            ("ts:a -> ", 1),
            ("ts:a b -> ts:c", 1),
        ] {
            assert_eq!(Policy::parse(text).unwrap_err().line, line, "{text:?}");
        }
    }
}
