//! CEL authorization rules (`policies.mcpAuthorization.rules`).
//!
//! Where `allowTools` / `denyTools` decide by tool name alone, a rule is an
//! expression over the call *and* the caller:
//!
//! ```yaml
//! mcpAuthorization:
//!   rules:
//!     - 'mcp.tool.name == "echo"'
//!     - 'jwt.sub == "test-user" && mcp.tool.name == "get-sum"'
//! ```
//!
//! # The name a rule sees is the unqualified one
//!
//! `mcp.tool.name` is the tool's own name on its target and `mcp.tool.target`
//! is the target it came from — the pair *before* federation joins them. So a
//! tool federated as `everything_echo` is `mcp.tool.name == "echo"` with
//! `mcp.tool.target == "everything"`, and a rule written against the federated
//! name never fires.
//!
//! That is upstream's split, and it is the more useful one: a rule can name a
//! tool without knowing what the gateway will prefix it with. It is also the
//! opposite of `allowTools`/`denyTools`, which match the federated name so one
//! route can ban a tool on one target while leaving the same name on another
//! alone. Both are in this file's tests, side by side, because the difference
//! is exactly the sort of thing that makes a policy silently not apply.
//!
//! # A failed expression is false
//!
//! An expression that cannot be evaluated — an absent `jwt` on an unauthenticated
//! route, a claim that is not there, a type mismatch — counts as false rather
//! than aborting the call. That is upstream's behaviour and it is safe for
//! `allow` and `require`, which fail towards refusing.
//!
//! It is *not* safe for `deny`: a `deny` that errors permits the call. This is
//! why `require` exists and why upstream's own schema recommends it over
//! `deny`. Both are supported here; the docs say which to reach for.

use agentgateway_config::AuthorizationRule;
use cel::{Context, Program};

/// A rule that does not compile.
#[derive(Debug, thiserror::Error)]
#[error("{at}: invalid CEL expression `{expression}`: {source}")]
pub struct RuleError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The expression that failed.
    pub expression: String,
    /// Why it failed.
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
}

/// A compiled expression, kept alongside its source for log lines.
struct Rule {
    expression: String,
    program: Program,
}

impl std::fmt::Debug for Rule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Rule").field(&self.expression).finish()
    }
}

impl Rule {
    /// Evaluate against a call, treating any failure as false.
    fn holds(&self, context: &Context<'_>) -> bool {
        match self.program.execute(context) {
            Ok(cel::Value::Bool(value)) => value,
            Ok(other) => {
                tracing::debug!(
                    expression = %self.expression,
                    result = ?other,
                    "a rule produced a non-boolean; treating it as false"
                );
                false
            }
            Err(err) => {
                // Routine, not exceptional: `jwt.sub == ...` on a route with no
                // token cannot evaluate, and the right answer there is "this
                // rule does not match", not a 500.
                tracing::debug!(
                    expression = %self.expression,
                    %err,
                    "a rule could not be evaluated; treating it as false"
                );
                false
            }
        }
    }
}

/// What a rule is evaluated against.
#[derive(Debug, Clone, Copy)]
pub struct ToolCall<'a> {
    /// The target the tool belongs to.
    pub target: &'a str,
    /// The tool's own name on that target, unqualified.
    pub tool: &'a str,
    /// Claims from the verified token, absent when the route has no `jwtAuth`.
    pub claims: Option<&'a serde_json::Value>,
}

/// A route's compiled `rules`.
#[derive(Debug, Default)]
pub struct RuleSet {
    allow: Vec<Rule>,
    deny: Vec<Rule>,
    require: Vec<Rule>,
}

impl RuleSet {
    /// Compile a route's rules.
    ///
    /// A bad expression stops the gateway booting. The alternative — skipping
    /// it — turns a typo in an `allow` rule into a route that denies
    /// everything, or a typo in a `deny` rule into one that denies nothing.
    pub fn new(rules: &[AuthorizationRule], at: &str) -> Result<Self, RuleError> {
        let mut set = RuleSet::default();

        for (i, rule) in rules.iter().enumerate() {
            let expression = rule.expression();
            let program = Program::compile(expression).map_err(|source| RuleError {
                at: format!("{at}.rules[{i}]"),
                expression: expression.to_string(),
                source: Box::new(source),
            })?;
            let compiled = Rule {
                expression: expression.to_string(),
                program,
            };
            match rule {
                AuthorizationRule::Allow(_) => set.allow.push(compiled),
                AuthorizationRule::Deny(_) => set.deny.push(compiled),
                AuthorizationRule::Require(_) => set.require.push(compiled),
            }
        }

        Ok(set)
    }

    /// Whether this route carries any rules at all.
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.deny.is_empty() && self.require.is_empty()
    }

    /// Whether the rules permit this call.
    ///
    /// The order is upstream's, and each step earns its place:
    ///
    /// 1. No rules at all permits — otherwise adding the `rules` key with an
    ///    empty list would take the route offline.
    /// 2. Any `deny` that holds refuses, ahead of everything else, so an allow
    ///    cannot be written to outrank a deny.
    /// 3. Every `require` must hold.
    /// 4. Any `allow` that holds permits.
    /// 5. Otherwise: permit only if there were no `allow` rules to satisfy.
    ///    A set of pure `deny` rules is a deny-list, so what it does not name
    ///    is permitted; the moment one `allow` exists the set becomes an
    ///    allow-list and an unmatched call is refused.
    pub fn permits(&self, call: ToolCall<'_>) -> bool {
        if self.is_empty() {
            return true;
        }

        let context = match Self::context(call) {
            Some(context) => context,
            // Binding cannot fail for the values we construct, but refusing
            // beats guessing if it ever does.
            None => return false,
        };

        if self.deny.iter().any(|rule| rule.holds(&context)) {
            return false;
        }
        if !self.require.iter().all(|rule| rule.holds(&context)) {
            return false;
        }
        if self.allow.iter().any(|rule| rule.holds(&context)) {
            return true;
        }
        self.allow.is_empty()
    }

    /// Bind `mcp` and `jwt` for one call.
    ///
    /// `jwt` is left unbound when there is no token rather than bound to null,
    /// so `jwt.sub == "x"` fails to resolve and reads as false. Binding null
    /// would make `jwt.sub` an error too, but leaving it out also keeps
    /// `has(jwt)` honest.
    fn context(call: ToolCall<'_>) -> Option<Context<'static>> {
        let mut context = Context::default();
        let mcp = serde_json::json!({
            "tool": { "name": call.tool, "target": call.target },
        });
        context.add_variable("mcp", mcp).ok()?;
        if let Some(claims) = call.claims {
            context.add_variable("jwt", claims.clone()).ok()?;
        }
        Some(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rules(rules: Vec<AuthorizationRule>) -> RuleSet {
        RuleSet::new(&rules, "test").expect("rules should compile")
    }

    fn allow(expression: &str) -> AuthorizationRule {
        AuthorizationRule::Allow(expression.to_string())
    }

    fn call<'a>(
        target: &'a str,
        tool: &'a str,
        claims: Option<&'a serde_json::Value>,
    ) -> ToolCall<'a> {
        ToolCall {
            target,
            tool,
            claims,
        }
    }

    #[test]
    fn no_rules_permits_everything() {
        let set = rules(Vec::new());
        assert!(set.is_empty());
        assert!(set.permits(call("everything", "echo", None)));
    }

    #[test]
    fn an_allow_list_refuses_what_it_does_not_name() {
        let set = rules(vec![allow(r#"mcp.tool.name == "echo""#)]);
        assert!(set.permits(call("everything", "echo", None)));
        assert!(
            !set.permits(call("everything", "delete", None)),
            "once one allow rule exists the set is an allow-list"
        );
    }

    #[test]
    fn a_rule_sees_the_unqualified_name_not_the_federated_one() {
        // A rule written against `everything_echo` would never fire, which is
        // the failure this test exists to pin down.
        let set = rules(vec![allow(r#"mcp.tool.name == "echo""#)]);
        assert!(set.permits(call("everything", "echo", None)));

        let federated = rules(vec![allow(r#"mcp.tool.name == "everything_echo""#)]);
        assert!(!federated.permits(call("everything", "echo", None)));
    }

    #[test]
    fn a_rule_can_name_the_target() {
        let set = rules(vec![allow(r#"mcp.tool.target == "safe""#)]);
        assert!(set.permits(call("safe", "anything", None)));
        assert!(!set.permits(call("risky", "anything", None)));
    }

    #[test]
    fn a_deny_list_alone_permits_everything_else() {
        // Otherwise writing one deny rule would silently take the route
        // offline, which is the same reading allowTools/denyTools use.
        let set = rules(vec![AuthorizationRule::Deny(
            r#"mcp.tool.name == "delete""#.into(),
        )]);
        assert!(!set.permits(call("t", "delete", None)));
        assert!(set.permits(call("t", "echo", None)));
    }

    #[test]
    fn deny_beats_allow() {
        let set = rules(vec![
            allow("true"),
            AuthorizationRule::Deny(r#"mcp.tool.name == "delete""#.into()),
        ]);
        assert!(set.permits(call("t", "echo", None)));
        assert!(
            !set.permits(call("t", "delete", None)),
            "matching both must resolve to denied"
        );
    }

    #[test]
    fn every_require_has_to_hold() {
        let set = rules(vec![
            AuthorizationRule::Require(r#"jwt.sub == "u1""#.into()),
            AuthorizationRule::Require(r#"mcp.tool.target == "safe""#.into()),
        ]);
        let claims = json!({"sub": "u1"});
        assert!(set.permits(call("safe", "echo", Some(&claims))));
        assert!(
            !set.permits(call("risky", "echo", Some(&claims))),
            "one failing require is enough to refuse"
        );
    }

    #[test]
    fn a_require_refuses_when_it_cannot_be_evaluated() {
        // This is the whole reason to prefer `require` over `deny`: with no
        // token there is no `jwt` to read, and the safe answer is to refuse.
        let set = rules(vec![AuthorizationRule::Require(
            r#"jwt.sub == "u1""#.into(),
        )]);
        assert!(!set.permits(call("t", "echo", None)));
    }

    #[test]
    fn a_deny_permits_when_it_cannot_be_evaluated() {
        // The documented footgun, pinned so it cannot change by accident: a
        // `deny` that errors lets the call through. Written as `require` the
        // same intent refuses -- see the test above.
        let set = rules(vec![AuthorizationRule::Deny(
            r#"jwt.role == "banned""#.into(),
        )]);
        assert!(
            set.permits(call("t", "echo", None)),
            "an unevaluable deny does not deny; this is why require exists"
        );
    }

    #[test]
    fn a_claim_is_matched_when_the_token_carries_it() {
        let set = rules(vec![allow(
            r#"jwt.sub == "test-user" && mcp.tool.name == "get-sum""#,
        )]);
        let claims = json!({"sub": "test-user"});
        assert!(set.permits(call("t", "get-sum", Some(&claims))));

        let other = json!({"sub": "someone-else"});
        assert!(!set.permits(call("t", "get-sum", Some(&other))));
    }

    #[test]
    fn a_nested_claim_resolves() {
        let set = rules(vec![allow(
            r#"mcp.tool.name == "get-env" && jwt.nested.key == "value""#,
        )]);
        let claims = json!({"nested": {"key": "value"}});
        assert!(set.permits(call("t", "get-env", Some(&claims))));

        let wrong = json!({"nested": {"key": "other"}});
        assert!(!set.permits(call("t", "get-env", Some(&wrong))));
    }

    #[test]
    fn an_allow_needing_a_claim_refuses_an_unauthenticated_caller() {
        let set = rules(vec![allow(r#"jwt.sub == "test-user""#)]);
        assert!(
            !set.permits(call("t", "echo", None)),
            "an allow that cannot be evaluated does not allow"
        );
    }

    #[test]
    fn a_non_boolean_rule_is_false_rather_than_an_error() {
        let set = rules(vec![allow("mcp.tool.name")]);
        assert!(!set.permits(call("t", "echo", None)));
    }

    #[test]
    fn a_bad_expression_names_where_it_came_from() {
        let err = RuleSet::new(
            &[allow("mcp.tool.name ==")],
            "binds[0].listeners[0].routes[0].policies.mcpAuthorization",
        )
        .expect_err("should not compile");
        assert!(
            err.to_string().contains("mcpAuthorization.rules[0]"),
            "got: {err}"
        );
    }
}
