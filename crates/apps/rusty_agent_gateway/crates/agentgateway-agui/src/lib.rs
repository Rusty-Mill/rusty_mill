//! The `agui` route policy: gate AG-UI runs with CEL, deny by default, and
//! write an audit record before the run is forwarded and another after it
//! ends.
//!
//! ```yaml
//! policies:
//!   jwtAuth: { issuer: "...", jwks: { file: "jwks.json" } }
//!   agui:
//!     rules:
//!       - 'jwt.sub == "alice"'
//!       - { require: 'agui.tools.all(t, t in ["create_task", "confirm"])' }
//!       - { deny: 'agui.lastUserMessage.contains("rm -rf")' }
//! backends:
//!   - host: "127.0.0.1:9000"
//! ```
//!
//! A `POST` on the route is read as a `RunAgentInput` and the rules decide
//! whether it reaches the agent. Anything else on the route is proxied
//! untouched.
//!
//! # Deny by default
//!
//! This is the one place the gateway refuses in silence. `mcpAuthorization`
//! permits when it has no rules, so adding the key cannot take a route
//! offline. An AG-UI run is different: it is the action, not a listing of
//! what actions exist, and it costs model time and may call tools. So a
//! route that carries `agui` refuses every run unless an `allow` rule says
//! otherwise: no rules refuse, pure `deny` rules refuse what they do not
//! name, and `require` narrows what an `allow` permits. Precedence is
//! otherwise `mcpAuthorization`'s: a `deny` that holds wins, every
//! `require` must hold, then any `allow` that holds permits.
//!
//! # What a rule sees
//!
//! - `agui.threadId`, `agui.runId`, `agui.parentRunId` (absent when not
//!   sent), `agui.messages` (the count), `agui.lastUserMessage` (its text,
//!   `""` when there is none), `agui.tools` (the names the client offers),
//!   `agui.forwardedProps`.
//! - `request.method`, `request.path`, `request.headers` (lowercase names).
//! - `jwt`, the verified claims, unbound when the route has no `jwtAuth`,
//!   so `jwt.sub == "x"` fails to evaluate and reads as false.
//!
//! An expression that fails to evaluate is false, as for MCP; prefer
//! `require` to `deny` for the same reason.

pub mod audit;

use agentgateway_config::{AguiPolicy, AuthorizationRule};
use cel::{Context, Program};
use http::{HeaderMap, StatusCode};
use rusty_agui::{Message, RunAgentInput};

pub use audit::{AuditedBody, RunReport};

/// A rule that does not compile, which stops the gateway booting.
#[derive(Debug, thiserror::Error)]
#[error("{at}: invalid CEL expression `{expression}`: {source}")]
pub struct AguiError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The expression that failed.
    pub expression: String,
    /// Why it failed.
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
}

/// A compiled expression, kept with its source for log lines.
struct Rule {
    expression: String,
    program: Program,
}

impl Rule {
    fn holds(&self, context: &Context<'_>) -> bool {
        match self.program.execute(context) {
            Ok(cel::Value::Bool(value)) => value,
            Ok(other) => {
                tracing::debug!(expression = %self.expression, result = ?other, "a rule produced a non-boolean; treating it as false");
                false
            }
            Err(err) => {
                tracing::debug!(expression = %self.expression, %err, "a rule could not be evaluated; treating it as false");
                false
            }
        }
    }
}

/// The request a run arrived on: what the rules may read besides the body.
#[derive(Debug, Clone, Copy)]
pub struct RunRequest<'a> {
    /// The HTTP method.
    pub method: &'a http::Method,
    /// The request path.
    pub path: &'a str,
    /// The request headers.
    pub headers: &'a HeaderMap,
    /// Claims from the verified token, absent when the route has no `jwtAuth`.
    pub claims: Option<&'a serde_json::Value>,
}

/// What the gateway knows about a run once it has read the input: the
/// fields every audit record carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    /// The thread the run belongs to.
    pub thread_id: String,
    /// The run's id.
    pub run_id: String,
    /// The tools the client offered, by name.
    pub tools: Vec<String>,
    /// How many messages the thread carried.
    pub messages: usize,
    /// `jwt.sub`, when there is a token with one.
    pub subject: Option<String>,
}

/// The gate's answer for one request.
#[derive(Debug)]
pub enum Decision {
    /// Forward the run.
    Permitted(RunSummary),
    /// Answer with this status and body; the agent never sees the run.
    Refused {
        /// The run, when the body could be read as one.
        summary: Option<RunSummary>,
        /// Why, for the audit record.
        reason: String,
        /// The HTTP status to answer with.
        status: StatusCode,
    },
    /// Not a run (not a `POST`): proxy it untouched.
    NotARun,
}

/// A route's compiled `agui` policy.
pub struct AguiGateway {
    allow: Vec<Rule>,
    deny: Vec<Rule>,
    require: Vec<Rule>,
}

impl std::fmt::Debug for AguiGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AguiGateway")
            .field("allow", &self.allow.len())
            .field("deny", &self.deny.len())
            .field("require", &self.require.len())
            .finish()
    }
}

impl AguiGateway {
    /// Compile a route's rules. A bad expression is an error, so a typo
    /// cannot quietly become a route that refuses (or permits) everything.
    pub fn new(policy: &AguiPolicy, at: &str) -> Result<Self, AguiError> {
        let mut gateway = AguiGateway {
            allow: Vec::new(),
            deny: Vec::new(),
            require: Vec::new(),
        };
        for (i, rule) in policy.rules.iter().enumerate() {
            let expression = rule.expression();
            let program = Program::compile(expression).map_err(|source| AguiError {
                at: format!("{at}.rules[{i}]"),
                expression: expression.to_string(),
                source: Box::new(source),
            })?;
            let compiled = Rule {
                expression: expression.to_string(),
                program,
            };
            match rule {
                AuthorizationRule::Allow(_) => gateway.allow.push(compiled),
                AuthorizationRule::Deny(_) => gateway.deny.push(compiled),
                AuthorizationRule::Require(_) => gateway.require.push(compiled),
            }
        }
        Ok(gateway)
    }

    /// Decide one request. `body` is the whole request body.
    pub fn check(&self, request: RunRequest<'_>, body: &[u8]) -> Decision {
        if request.method != http::Method::POST {
            return Decision::NotARun;
        }
        let input = match std::str::from_utf8(body)
            .map_err(|e| e.to_string())
            .and_then(|text| RunAgentInput::from_json(text).map_err(|e| e.to_string()))
        {
            Ok(input) => input,
            Err(reason) => {
                return Decision::Refused {
                    summary: None,
                    reason: format!("not a RunAgentInput: {reason}"),
                    status: StatusCode::BAD_REQUEST,
                };
            }
        };
        let summary = summarize(&input, request.claims);
        let refuse = |reason: &str| Decision::Refused {
            summary: Some(summary.clone()),
            reason: reason.into(),
            status: StatusCode::FORBIDDEN,
        };
        let Some(context) = context(&input, request) else {
            return refuse("the rule context could not be built");
        };
        if let Some(rule) = self.deny.iter().find(|r| r.holds(&context)) {
            return refuse(&format!("deny rule matched: {}", rule.expression));
        }
        if let Some(rule) = self.require.iter().find(|r| !r.holds(&context)) {
            return refuse(&format!("require rule failed: {}", rule.expression));
        }
        if self.allow.iter().any(|r| r.holds(&context)) {
            return Decision::Permitted(summary);
        }
        refuse(if self.allow.is_empty() {
            "no allow rule on this route"
        } else {
            "no allow rule matched"
        })
    }
}

fn summarize(input: &RunAgentInput, claims: Option<&serde_json::Value>) -> RunSummary {
    RunSummary {
        thread_id: input.thread_id.clone(),
        run_id: input.run_id.clone(),
        tools: input.tools.iter().map(|t| t.name.clone()).collect(),
        messages: input.messages.len(),
        subject: claims
            .and_then(|c| c.get("sub"))
            .and_then(serde_json::Value::as_str)
            .map(String::from),
    }
}

fn last_user_message(input: &RunAgentInput) -> String {
    input
        .messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::User { content, .. } => Some(content.text()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Bind `agui`, `request` and `jwt` for one run.
fn context(input: &RunAgentInput, request: RunRequest<'_>) -> Option<Context<'static>> {
    let mut context = Context::default();
    // rusty_agui is on rusty_json; the rule engine takes serde_json. The
    // forwarded props are the one open-ended value, so they cross over as
    // text.
    let forwarded: serde_json::Value =
        serde_json::from_str(&input.forwarded_props.to_json_string())
            .unwrap_or(serde_json::Value::Null);
    let mut agui = serde_json::json!({
        "threadId": input.thread_id,
        "runId": input.run_id,
        "messages": input.messages.len(),
        "lastUserMessage": last_user_message(input),
        "tools": input.tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        "forwardedProps": forwarded,
    });
    if let Some(parent) = &input.parent_run_id {
        agui["parentRunId"] = serde_json::Value::String(parent.clone());
    }
    context.add_variable("agui", agui).ok()?;

    let headers: serde_json::Map<String, serde_json::Value> = request
        .headers
        .iter()
        .filter_map(|(name, value)| {
            value.to_str().ok().map(|v| {
                (
                    name.as_str().to_string(),
                    serde_json::Value::String(v.to_string()),
                )
            })
        })
        .collect();
    context
        .add_variable(
            "request",
            serde_json::json!({
                "method": request.method.as_str(),
                "path": request.path,
                "headers": headers,
            }),
        )
        .ok()?;
    if let Some(claims) = request.claims {
        context.add_variable("jwt", claims.clone()).ok()?;
    }
    Some(context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn policy(rules: &[AuthorizationRule]) -> AguiGateway {
        AguiGateway::new(
            &AguiPolicy {
                rules: rules.to_vec(),
            },
            "route",
        )
        .expect("rules compile")
    }

    fn run(text: &str, tools: &[&str]) -> Vec<u8> {
        let tools: Vec<_> = tools
            .iter()
            .map(|t| json!({"name": t, "description": "", "parameters": {"type": "object"}}))
            .collect();
        json!({
            "threadId": "t", "runId": "r",
            "messages": [{"id": "u", "role": "user", "content": text}],
            "tools": tools,
            "forwardedProps": {"tenant": "acme"}
        })
        .to_string()
        .into_bytes()
    }

    fn post<'a>(headers: &'a HeaderMap, claims: Option<&'a serde_json::Value>) -> RunRequest<'a> {
        RunRequest {
            method: &http::Method::POST,
            path: "/agent",
            headers,
            claims,
        }
    }

    fn refused(decision: Decision) -> (StatusCode, String) {
        match decision {
            Decision::Refused { status, reason, .. } => (status, reason),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn refuses_by_default_and_permits_only_on_an_allow() {
        let headers = HeaderMap::new();
        let (status, reason) = refused(policy(&[]).check(post(&headers, None), &run("hi", &[])));
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(reason, "no allow rule on this route");

        let deny_only = policy(&[AuthorizationRule::Deny("agui.messages > 100".into())]);
        assert_eq!(
            refused(deny_only.check(post(&headers, None), &run("hi", &[]))).1,
            "no allow rule on this route"
        );

        let allow = policy(&[AuthorizationRule::Allow("agui.threadId == \"t\"".into())]);
        assert!(matches!(
            allow.check(post(&headers, None), &run("hi", &["create_task"])),
            Decision::Permitted(RunSummary { ref tools, messages: 1, .. }) if tools == &["create_task".to_string()]
        ));
        let other = policy(&[AuthorizationRule::Allow(
            "agui.threadId == \"other\"".into(),
        )]);
        assert_eq!(
            refused(other.check(post(&headers, None), &run("hi", &[]))).1,
            "no allow rule matched"
        );
    }

    #[test]
    fn deny_wins_and_require_narrows() {
        let headers = HeaderMap::new();
        let gate = policy(&[
            AuthorizationRule::Allow("true".into()),
            AuthorizationRule::Deny("agui.lastUserMessage.contains(\"rm -rf\")".into()),
            AuthorizationRule::Require("agui.tools.all(t, t in [\"create_task\"])".into()),
        ]);
        assert!(matches!(
            gate.check(post(&headers, None), &run("add milk", &["create_task"])),
            Decision::Permitted(_)
        ));
        assert!(
            refused(gate.check(post(&headers, None), &run("rm -rf /", &["create_task"])))
                .1
                .starts_with("deny rule matched")
        );
        assert!(
            refused(gate.check(post(&headers, None), &run("add milk", &["shell"])))
                .1
                .starts_with("require rule failed")
        );
    }

    #[test]
    fn rules_see_the_caller_the_request_and_forwarded_props() {
        let mut headers = HeaderMap::new();
        headers.insert("x-tenant", "acme".parse().expect("header"));
        let claims = json!({"sub": "alice"});
        let gate = policy(&[AuthorizationRule::Allow(
            "jwt.sub == \"alice\" && request.headers[\"x-tenant\"] == agui.forwardedProps.tenant && request.path == \"/agent\"".into(),
        )]);
        match gate.check(post(&headers, Some(&claims)), &run("hi", &[])) {
            Decision::Permitted(summary) => assert_eq!(summary.subject.as_deref(), Some("alice")),
            other => panic!("{other:?}"),
        }
        // No token: `jwt` is unbound, the rule fails to evaluate, the run is refused.
        assert_eq!(
            refused(gate.check(post(&headers, None), &run("hi", &[]))).1,
            "no allow rule matched"
        );
    }

    #[test]
    fn a_non_run_passes_and_a_bad_body_is_a_400() {
        let headers = HeaderMap::new();
        let gate = policy(&[AuthorizationRule::Allow("true".into())]);
        let get = RunRequest {
            method: &http::Method::GET,
            path: "/agent",
            headers: &headers,
            claims: None,
        };
        assert!(matches!(gate.check(get, b""), Decision::NotARun));
        let (status, reason) = refused(gate.check(post(&headers, None), b"{\"runId\":\"r\"}"));
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(reason.contains("threadId"), "{reason}");
    }

    #[test]
    fn a_bad_expression_fails_to_build() {
        let err = AguiGateway::new(
            &AguiPolicy {
                rules: vec![AuthorizationRule::Allow("agui.threadId ==".into())],
            },
            "route",
        )
        .expect_err("should not compile");
        assert_eq!(err.at, "route.rules[0]");
    }
}
