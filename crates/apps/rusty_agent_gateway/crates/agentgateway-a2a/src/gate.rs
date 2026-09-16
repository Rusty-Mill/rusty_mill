//! Method-level authorization for A2A traffic.
//!
//! The A2A analogue of MCP's tool gate: an A2A call names its operation in the
//! JSON-RPC `method` field, so a route can permit `message/send` while
//! refusing `tasks/cancel`.
//!
//! As with MCP, the check runs on the call itself rather than on any
//! advertisement of it. An agent card is a description, not an access control
//! list, and a caller that already knows a method name never has to read one.

use agentgateway_config::A2aPolicy;
use regex::Regex;

/// A method pattern that does not compile.
#[derive(Debug, thiserror::Error)]
#[error("{at}: invalid regex `{pattern}`: {source}")]
pub struct GateError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The pattern that failed.
    pub pattern: String,
    /// Why it failed.
    #[source]
    pub source: Box<regex::Error>,
}

/// Which JSON-RPC methods a route permits.
#[derive(Debug, Clone, Default)]
pub struct MethodGate {
    allow: Vec<Regex>,
    deny: Vec<Regex>,
}

impl MethodGate {
    /// Compile a route's method rules.
    pub fn new(policy: &A2aPolicy, at: &str) -> Result<Self, GateError> {
        let compile = |patterns: &[String], field: &str| -> Result<Vec<Regex>, GateError> {
            patterns
                .iter()
                .enumerate()
                .map(|(i, pattern)| {
                    Regex::new(pattern).map_err(|source| GateError {
                        at: format!("{at}.a2a.{field}[{i}]"),
                        pattern: pattern.clone(),
                        source: Box::new(source),
                    })
                })
                .collect()
        };

        Ok(MethodGate {
            allow: compile(&policy.allow_methods, "allowMethods")?,
            deny: compile(&policy.deny_methods, "denyMethods")?,
        })
    }

    /// Whether `method` may be invoked on this route.
    ///
    /// Deny wins, and an empty allow list means "everything not denied" — the
    /// same reading as the MCP gate, because a `denyMethods` that silently
    /// switched the route to deny-all would be useless on its own.
    pub fn permits(&self, method: &str) -> bool {
        if self.deny.iter().any(|re| re.is_match(method)) {
            return false;
        }
        self.allow.is_empty() || self.allow.iter().any(|re| re.is_match(method))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(allow: &[&str], deny: &[&str]) -> MethodGate {
        MethodGate::new(
            &A2aPolicy {
                allow_methods: allow.iter().map(|s| s.to_string()).collect(),
                deny_methods: deny.iter().map(|s| s.to_string()).collect(),
                agent_card: None,
            },
            "test",
        )
        .expect("patterns should compile")
    }

    #[test]
    fn no_rules_permits_every_method() {
        let gate = gate(&[], &[]);
        for method in ["message/send", "tasks/get", "tasks/cancel"] {
            assert!(gate.permits(method));
        }
    }

    #[test]
    fn a_deny_list_alone_permits_everything_else() {
        // Otherwise writing one rule would silently take the route offline.
        let gate = gate(&[], &["^tasks/cancel$"]);
        assert!(!gate.permits("tasks/cancel"));
        assert!(gate.permits("message/send"));
        assert!(gate.permits("tasks/get"));
    }

    #[test]
    fn an_allow_list_excludes_what_it_does_not_name() {
        let gate = gate(&["^message/"], &[]);
        assert!(gate.permits("message/send"));
        assert!(gate.permits("message/stream"));
        assert!(!gate.permits("tasks/cancel"));
    }

    #[test]
    fn deny_beats_allow() {
        let gate = gate(&["^tasks/"], &["^tasks/cancel$"]);
        assert!(gate.permits("tasks/get"));
        assert!(
            !gate.permits("tasks/cancel"),
            "matching both rules must resolve to denied"
        );
    }

    #[test]
    fn a_bad_pattern_names_where_it_came_from() {
        let err = MethodGate::new(
            &A2aPolicy {
                allow_methods: vec!["[".into()],
                ..Default::default()
            },
            "binds[0].listeners[0].routes[0].policies",
        )
        .expect_err("should not compile");
        assert!(err.to_string().contains("allowMethods[0]"), "got: {err}");
    }
}
