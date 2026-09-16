//! Tool visibility and authorization.
//!
//! Two independent gates sit in front of a federated tool:
//!
//! - **Target filters** (`backends[].mcp.targets[].filters`) decide which of a
//!   target's own tools the gateway federates at all.
//! - **Route authorization** (`policies.mcpAuthorization`) decides which
//!   federated names a caller on this route may use.
//!
//! Both are enforced on `tools/call`, not only on `tools/list`. Filtering the
//! listing alone is security theatre: nothing stops a client from calling a
//! name it was never shown, and a tool hidden from the catalogue but still
//! callable is strictly worse than one that was never hidden, because the
//! operator believes it is gone.

use agentgateway_config::{FilterAction, McpAuthorization, ToolFilter};
use regex::Regex;

/// Failure to compile a gate's patterns.
#[derive(Debug, thiserror::Error)]
#[error("{at}: invalid regex `{pattern}`: {source}")]
pub struct GateError {
    /// Where in the configuration the pattern came from.
    pub at: String,
    /// The pattern that failed to compile.
    pub pattern: String,
    /// Why it failed.
    #[source]
    pub source: Box<regex::Error>,
}

fn compile(pattern: &str, at: &str) -> Result<Regex, GateError> {
    Regex::new(pattern).map_err(|source| GateError {
        at: at.to_string(),
        pattern: pattern.to_string(),
        source: Box::new(source),
    })
}

/// Which of a target's own tools are federated.
#[derive(Debug, Clone, Default)]
pub struct TargetFilter {
    allow: Vec<Regex>,
    deny: Vec<Regex>,
}

impl TargetFilter {
    /// Compile a target's filter list.
    pub fn new(filters: &[ToolFilter], at: &str) -> Result<Self, GateError> {
        let mut allow = Vec::new();
        let mut deny = Vec::new();
        for (i, filter) in filters.iter().enumerate() {
            let regex = compile(&filter.matcher, &format!("{at}.filters[{i}]"))?;
            match filter.action {
                FilterAction::Allow => allow.push(regex),
                FilterAction::Deny => deny.push(regex),
            }
        }
        Ok(TargetFilter { allow, deny })
    }

    /// Whether an unqualified tool name survives this target's filters.
    pub fn permits(&self, tool: &str) -> bool {
        decide(&self.allow, &self.deny, tool)
    }
}

/// Which federated names a caller on this route may use.
#[derive(Debug, Clone, Default)]
pub struct Authorization {
    allow: Vec<Regex>,
    deny: Vec<Regex>,
}

impl Authorization {
    /// Compile a route's `mcpAuthorization` policy.
    pub fn new(policy: &McpAuthorization, at: &str) -> Result<Self, GateError> {
        let mut allow = Vec::with_capacity(policy.allow_tools.len());
        for (i, pattern) in policy.allow_tools.iter().enumerate() {
            allow.push(compile(pattern, &format!("{at}.allowTools[{i}]"))?);
        }
        let mut deny = Vec::with_capacity(policy.deny_tools.len());
        for (i, pattern) in policy.deny_tools.iter().enumerate() {
            deny.push(compile(pattern, &format!("{at}.denyTools[{i}]"))?);
        }
        Ok(Authorization { allow, deny })
    }

    /// Whether a federated tool name may be called on this route.
    pub fn permits(&self, federated: &str) -> bool {
        decide(&self.allow, &self.deny, federated)
    }
}

/// Deny wins; an empty allow list means "everything not denied".
///
/// The alternative — an empty allow list denying everything — would make
/// `denyTools` alone useless, since writing one would silently switch the
/// route to deny-all.
fn decide(allow: &[Regex], deny: &[Regex], name: &str) -> bool {
    if deny.iter().any(|re| re.is_match(name)) {
        return false;
    }
    allow.is_empty() || allow.iter().any(|re| re.is_match(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentgateway_config::{FilterAction, ToolFilter};

    fn filter(rules: &[(FilterAction, &str)]) -> TargetFilter {
        let filters: Vec<ToolFilter> = rules
            .iter()
            .map(|(action, matcher)| ToolFilter {
                action: *action,
                matcher: (*matcher).to_string(),
            })
            .collect();
        TargetFilter::new(&filters, "test").expect("patterns should compile")
    }

    fn authorization(allow: &[&str], deny: &[&str]) -> Authorization {
        Authorization::new(
            &McpAuthorization {
                rules: Vec::new(),
                allow_tools: allow.iter().map(|s| s.to_string()).collect(),
                deny_tools: deny.iter().map(|s| s.to_string()).collect(),
            },
            "test",
        )
        .expect("patterns should compile")
    }

    #[test]
    fn no_rules_permits_everything() {
        assert!(filter(&[]).permits("anything"));
        assert!(authorization(&[], &[]).permits("anything"));
    }

    #[test]
    fn an_allow_list_excludes_what_it_does_not_name() {
        let gate = filter(&[(FilterAction::Allow, "^read_")]);
        assert!(gate.permits("read_file"));
        assert!(!gate.permits("write_file"));
    }

    #[test]
    fn a_deny_list_alone_permits_everything_else() {
        // If an empty allow list meant "deny all", writing only a deny rule
        // would silently take the whole target offline.
        let gate = filter(&[(FilterAction::Deny, "^delete_")]);
        assert!(!gate.permits("delete_repo"));
        assert!(gate.permits("read_file"));
    }

    #[test]
    fn deny_beats_allow() {
        let gate = filter(&[
            (FilterAction::Allow, "^file_"),
            (FilterAction::Deny, "_secret$"),
        ]);
        assert!(gate.permits("file_read"));
        assert!(
            !gate.permits("file_secret"),
            "matching both rules must resolve to denied"
        );
    }

    #[test]
    fn authorization_matches_the_federated_name() {
        // The route policy sees `github_delete_repo`, not `delete_repo` --
        // which is what lets one route ban a tool on one target while leaving
        // the same-named tool on another target alone.
        let gate = authorization(&[], &["^github_delete_"]);
        assert!(!gate.permits("github_delete_repo"));
        assert!(gate.permits("jira_delete_issue"));
    }

    #[test]
    fn a_bad_pattern_names_where_it_came_from() {
        let err = TargetFilter::new(
            &[ToolFilter {
                action: FilterAction::Allow,
                matcher: "[".into(),
            }],
            "backends[0].mcp.targets[0]",
        )
        .expect_err("should not compile");
        assert!(err.to_string().contains("backends[0].mcp.targets[0]"));
    }
}
