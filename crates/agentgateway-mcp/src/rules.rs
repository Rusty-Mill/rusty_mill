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
//!     - 'mcp.prompt.name == "summarize"'
//!     - 'mcp.resource.name.startsWith("memo:")'
//! ```
//!
//! Tools, prompts and resources are all gated the same way, and exactly one of
//! `mcp.tool`, `mcp.prompt` and `mcp.resource` is bound per call — see
//! [`Subject`], which is where that decision does the most work.
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

/// What kind of thing a rule is being asked about.
///
/// Exactly one of `mcp.tool`, `mcp.prompt` and `mcp.resource` is bound for any
/// one call, which is upstream's shape and it carries real weight: on a
/// `prompts/get`, the expression `mcp.tool.name == "echo"` does not resolve, so
/// it reads as **false** rather than as "not about tools, ignore me".
///
/// The consequence is worth stating plainly, because it is the one that
/// surprises people. A rule set written entirely as tool `allow` rules is an
/// allow-list that nothing in the prompt or resource space can satisfy, so it
/// refuses every prompt and resource. That is the safe direction — a rule set
/// that had never heard of prompts should not wave them through — but it does
/// mean adding prompts to a target behind an existing tool allow-list takes an
/// explicit rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject<'a> {
    /// A tool, by its own name on its target.
    Tool(&'a str),
    /// A prompt, by its own name on its target.
    Prompt(&'a str),
    /// A resource, by its URI as its target publishes it.
    ///
    /// Bound as `mcp.resource.name`, not `.uri` — upstream names the field for
    /// the shape it shares with tools and prompts rather than for what a
    /// resource happens to call its identifier.
    Resource(&'a str),
}

impl<'a> Subject<'a> {
    /// The CEL key this subject binds under.
    fn key(&self) -> &'static str {
        match self {
            Subject::Tool(_) => "tool",
            Subject::Prompt(_) => "prompt",
            Subject::Resource(_) => "resource",
        }
    }

    /// The name or URI itself.
    fn name(&self) -> &'a str {
        match self {
            Subject::Tool(name) | Subject::Prompt(name) | Subject::Resource(name) => name,
        }
    }

    /// What to call this in an error a caller will read.
    pub fn noun(&self) -> &'static str {
        match self {
            Subject::Tool(_) => "tool",
            Subject::Prompt(_) => "prompt",
            Subject::Resource(_) => "resource",
        }
    }
}

/// What a rule is evaluated against.
#[derive(Debug, Clone, Copy)]
pub struct Call<'a> {
    /// The target the subject belongs to.
    pub target: &'a str,
    /// What is being accessed.
    pub subject: Subject<'a>,
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
    pub fn permits(&self, call: Call<'_>) -> bool {
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
    fn context(call: Call<'_>) -> Option<Context<'static>> {
        let mut context = Context::default();
        // Only the subject's own key is bound. A tool rule asked about a prompt
        // finds no `mcp.tool`, fails to resolve, and reads as false.
        let mcp = serde_json::json!({
            call.subject.key(): {
                "name": call.subject.name(),
                "target": call.target,
            },
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

    pub(super) fn rules(rules: Vec<AuthorizationRule>) -> RuleSet {
        RuleSet::new(&rules, "test").expect("rules should compile")
    }

    fn allow(expression: &str) -> AuthorizationRule {
        AuthorizationRule::Allow(expression.to_string())
    }

    fn call<'a>(target: &'a str, tool: &'a str, claims: Option<&'a serde_json::Value>) -> Call<'a> {
        Call {
            target,
            subject: Subject::Tool(tool),
            claims,
        }
    }

    pub(super) fn about<'a>(target: &'a str, subject: Subject<'a>) -> Call<'a> {
        Call {
            target,
            subject,
            claims: None,
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

#[cfg(test)]
mod subject_tests {
    use super::tests::*;
    use super::*;
    use serde_json::json;

    fn deny(expression: &str) -> AuthorizationRule {
        AuthorizationRule::Deny(expression.to_string())
    }

    fn require(expression: &str) -> AuthorizationRule {
        AuthorizationRule::Require(expression.to_string())
    }

    #[test]
    fn a_prompt_rule_gates_prompts() {
        let set = rules(vec![AuthorizationRule::Allow(
            r#"mcp.prompt.name == "summarize""#.into(),
        )]);
        assert!(set.permits(about("alpha", Subject::Prompt("summarize"))));
        assert!(!set.permits(about("alpha", Subject::Prompt("leak"))));
    }

    #[test]
    fn a_resource_rule_matches_on_the_uri() {
        // `mcp.resource.name` holds the URI. Upstream names the field for the
        // shape it shares with tools and prompts, not for what a resource
        // calls its identifier.
        let set = rules(vec![AuthorizationRule::Allow(
            r#"mcp.resource.name.startsWith("memo:")"#.into(),
        )]);
        assert!(set.permits(about("alpha", Subject::Resource("memo:insights"))));
        assert!(!set.permits(about("alpha", Subject::Resource("file:///etc/passwd"))));
    }

    #[test]
    fn a_rule_sees_the_unqualified_name_for_every_subject() {
        // Same split as tools: the pair before federation joins them.
        let set = rules(vec![AuthorizationRule::Allow(
            r#"mcp.prompt.target == "alpha" && mcp.prompt.name == "summarize""#.into(),
        )]);
        assert!(set.permits(about("alpha", Subject::Prompt("summarize"))));
        assert!(
            !set.permits(about("alpha", Subject::Prompt("alpha_summarize"))),
            "a rule written against the federated name must not fire"
        );
    }

    #[test]
    fn only_the_subjects_own_key_is_bound() {
        // The decision that carries the most weight. On a prompt call
        // `mcp.tool` does not resolve, so a tool rule reads as false rather
        // than as "not about tools, ignore me".
        let set = rules(vec![AuthorizationRule::Allow(
            r#"mcp.tool.name == "echo""#.into(),
        )]);
        assert!(set.permits(about("alpha", Subject::Tool("echo"))));
        assert!(!set.permits(about("alpha", Subject::Prompt("echo"))));
        assert!(!set.permits(about("alpha", Subject::Resource("echo"))));
    }

    #[test]
    fn a_tool_allow_list_refuses_prompts_and_resources() {
        // The consequence of the above, and the one that surprises people:
        // adding prompts to a target behind an existing tool allow-list takes
        // an explicit rule. It is the safe direction -- a rule set that had
        // never heard of prompts should not wave them through -- but it is a
        // real behaviour change and it is pinned here.
        let set = rules(vec![AuthorizationRule::Allow(
            r#"mcp.tool.name == "echo""#.into(),
        )]);
        assert!(!set.permits(about("alpha", Subject::Prompt("anything"))));
        assert!(!set.permits(about("alpha", Subject::Resource("memo:x"))));
    }

    #[test]
    fn a_pure_deny_list_still_permits_other_subjects() {
        // The mirror image: a deny-list names what is refused, so a prompt it
        // does not name survives. Nothing here depends on subject kind, which
        // is the point.
        let set = rules(vec![deny(r#"mcp.tool.name == "delete""#)]);
        assert!(set.permits(about("alpha", Subject::Prompt("anything"))));
        assert!(set.permits(about("alpha", Subject::Resource("memo:x"))));
        assert!(!set.permits(about("alpha", Subject::Tool("delete"))));
    }

    #[test]
    fn subjects_can_be_mixed_in_one_rule_set() {
        let set = rules(vec![
            AuthorizationRule::Allow(r#"mcp.tool.name == "echo""#.into()),
            AuthorizationRule::Allow(r#"mcp.prompt.name == "summarize""#.into()),
            AuthorizationRule::Allow(r#"mcp.resource.name == "memo:insights""#.into()),
        ]);
        assert!(set.permits(about("alpha", Subject::Tool("echo"))));
        assert!(set.permits(about("alpha", Subject::Prompt("summarize"))));
        assert!(set.permits(about("alpha", Subject::Resource("memo:insights"))));
        assert!(!set.permits(about("alpha", Subject::Tool("summarize"))));
    }

    #[test]
    fn a_require_applies_across_every_subject() {
        // A require is the way to write a rule that is genuinely about the
        // caller rather than about one kind of thing.
        let set = rules(vec![require(r#"jwt.role == "admin""#)]);
        let admin = json!({"role": "admin"});
        let viewer = json!({"role": "viewer"});

        for subject in [
            Subject::Tool("echo"),
            Subject::Prompt("summarize"),
            Subject::Resource("memo:x"),
        ] {
            assert!(set.permits(Call {
                target: "alpha",
                subject,
                claims: Some(&admin),
            }));
            assert!(!set.permits(Call {
                target: "alpha",
                subject,
                claims: Some(&viewer),
            }));
            assert!(
                !set.permits(Call {
                    target: "alpha",
                    subject,
                    claims: None,
                }),
                "an unevaluable require refuses, for every subject"
            );
        }
    }

    #[test]
    fn a_deny_that_cannot_be_evaluated_still_permits() {
        // Unchanged by widening the subject, and worth pinning again here:
        // this is why the docs recommend `require` over `deny`.
        let set = rules(vec![deny(r#"jwt.role == "banned""#)]);
        assert!(set.permits(about("alpha", Subject::Prompt("summarize"))));
    }

    #[test]
    fn a_subject_names_itself_for_error_messages() {
        assert_eq!(Subject::Tool("x").noun(), "tool");
        assert_eq!(Subject::Prompt("x").noun(), "prompt");
        assert_eq!(Subject::Resource("x").noun(), "resource");
    }
}
