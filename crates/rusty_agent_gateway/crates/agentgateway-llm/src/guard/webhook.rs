//! `promptGuard.webhook`: an external service asked whether text may pass.
//!
//! Where [`super`]'s `regex` rules decide from a pattern written down in
//! advance, this asks something that can change its mind — a classifier, a
//! policy service, a model.
//!
//! # The wire contract is upstream's, read from upstream's source
//!
//! It is not in agentgateway's published documentation: the guardrail pages
//! describe how to *configure* a webhook and link to an API reference that
//! does not render. The shapes here are taken from
//! `crates/agentgateway/src/llm/policy/webhook.rs` in the upstream repository,
//! so an existing webhook works against this gateway unchanged. That is the
//! whole point of the project, and guessing a contract would have quietly
//! broken it.
//!
//! The gateway `POST`s to `/request` and `/response` — overridable with a
//! `:path` header expression — with:
//!
//! ```json
//! {"body": {"messages": [{"role": "user", "content": "..."}]}}
//! {"body": {"choices": [{"message": {"role": "assistant", "content": "..."}}]}}
//! ```
//!
//! and reads back one `action`, distinguished by shape rather than by a tag:
//!
//! ```json
//! {"action": {"reason": "..."}}                                  // pass
//! {"action": {"body": {"messages": [...]}, "reason": "..."}}     // mask
//! {"action": {"body": "text", "status_code": 403}}               // reject
//! ```
//!
//! Untagged means order matters when reading: `mask` carries an *object*
//! `body` and `reject` a *string* one, so a reject cannot be mistaken for a
//! mask, and `pass` is what is left.
//!
//! # It fails closed
//!
//! A webhook that cannot be reached, times out, or answers with something
//! unreadable **refuses** the request unless `failureMode: failOpen` says
//! otherwise. This is a content control; a content control that waves traffic
//! through when its service is down is not one. Matching `extAuthz` and
//! `mcpGuardrails`, which made the same call for the same reason.

use std::collections::BTreeMap;
use std::time::Duration;

use agentgateway_config::{FailureMode, GuardWebhook};
use serde_json::{Value, json};

use super::Rejection;

/// Budget for one webhook call when the policy names none.
///
/// This sits in front of every request on the route, so a slow webhook is a
/// slow gateway. Longer than `extAuthz`'s 250ms because a guard may be running
/// a model rather than a lookup, and short enough that a hung one is noticed.
const TIMEOUT: Duration = Duration::from_secs(5);

/// A compiled `webhook` rule.
#[derive(Debug)]
pub struct Webhook {
    /// `/request` and `/response` are appended to this.
    base: String,
    /// Header name (or `:path`) to the CEL source that produces its value.
    headers: Vec<(String, String)>,
    /// Which of the caller's own headers travel along.
    forward: Vec<String>,
    fail_open: bool,
    client: reqwest::Client,
}

/// What a webhook said.
#[derive(Debug)]
pub enum Verdict {
    /// Nothing to do.
    Pass,
    /// Replace the text with what it sent back.
    Mask(Vec<String>),
    /// Refuse.
    Reject(Rejection),
}

impl Webhook {
    /// Compile one rule.
    pub fn new(config: &GuardWebhook) -> Self {
        Webhook {
            // `http://` because a guard webhook is an internal service and
            // upstream's `target.host` is a bare `host:port`. A TLS one is
            // reachable by spelling the scheme in the host.
            base: match config.target.host.starts_with("http") {
                true => config.target.host.trim_end_matches('/').to_string(),
                false => format!("http://{}", config.target.host.trim_end_matches('/')),
            },
            headers: config
                .headers
                .iter()
                .map(|(name, expression)| (name.to_string(), expression.to_string()))
                .collect(),
            forward: config
                .forward_header_matches
                .iter()
                .map(|name| name.to_ascii_lowercase())
                .collect::<Vec<String>>(),
            fail_open: config.failure_mode == FailureMode::FailOpen,
            client: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    /// Ask about a prompt.
    pub async fn check_messages(
        &self,
        messages: &[(String, String)],
        context: &Context,
    ) -> Verdict {
        let body = json!({"body": {"messages": messages
            .iter()
            .map(|(role, content)| json!({"role": role, "content": content}))
            .collect::<Vec<_>>()}});
        self.ask("request", body, context, "messages").await
    }

    /// Ask about an answer.
    pub async fn check_answer(&self, text: &str, context: &Context) -> Verdict {
        let body = json!({"body": {"choices": [
            {"message": {"role": "assistant", "content": text}}
        ]}});
        self.ask("response", body, context, "choices").await
    }

    async fn ask(&self, phase: &str, body: Value, context: &Context, key: &str) -> Verdict {
        let mut path = format!("/{phase}");
        let mut headers: Vec<(String, String)> = Vec::new();

        for (name, expression) in &self.headers {
            let Some(value) = context.eval(expression) else {
                // A header whose expression produced nothing is left off
                // rather than sent empty: an empty `x-tenant` is a claim that
                // there is no tenant, which is not what happened.
                tracing::debug!(header = %name, "a webhook header expression produced no value");
                continue;
            };
            // Pseudo-headers are instructions about the request rather than
            // headers on it. Only `:path` changes anything a webhook can see.
            match name.as_str() {
                ":path" => path = value,
                other if other.starts_with(':') => {
                    tracing::debug!(header = other, "ignoring an unsupported pseudo-header");
                }
                _ => headers.push((name.clone(), value)),
            }
        }

        for name in &self.forward {
            if let Some(value) = context.header(name) {
                headers.push((name.clone(), value.to_string()));
            }
        }

        let mut request = self.client.post(format!("{}{path}", self.base)).json(&body);
        for (name, value) in headers {
            request = request.header(name, value);
        }

        match self.send(request, key).await {
            Ok(verdict) => verdict,
            Err(reason) => {
                tracing::warn!(
                    %reason,
                    fail_open = self.fail_open,
                    "the guard webhook could not be consulted"
                );
                self.unreachable()
            }
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder, key: &str) -> Result<Verdict, String> {
        let response = request.send().await.map_err(|err| err.to_string())?;
        if !response.status().is_success() {
            // A webhook that answers 500 has not made a decision, so this is
            // the unreachable case rather than a refusal it chose.
            return Err(format!("it answered {}", response.status()));
        }
        let answered: Value = response.json().await.map_err(|err| err.to_string())?;
        Ok(read_action(&answered, key))
    }

    fn unreachable(&self) -> Verdict {
        if self.fail_open {
            return Verdict::Pass;
        }
        Verdict::Reject(Rejection {
            // 503, not 400: nothing decided this content was unacceptable, and
            // saying it was would send someone to inspect a prompt that is
            // fine when the real problem is a service being down.
            status: 503,
            headers: None,
            body: None,
        })
    }
}

/// Read the untagged action out of a webhook's answer.
///
/// `reject` before `mask` because the two are told apart by the *type* of
/// `body`: a string is a refusal message, an object is replacement content.
fn read_action(answered: &Value, key: &str) -> Verdict {
    let Some(action) = answered.get("action") else {
        // Upstream's schema requires it. A body without one has not answered
        // the question, so it is not a pass.
        tracing::warn!("a guard webhook answered without an `action`");
        return Verdict::Reject(Rejection {
            status: 503,
            headers: None,
            body: None,
        });
    };

    match action.get("body") {
        Some(Value::String(message)) => Verdict::Reject(Rejection {
            status: action
                .get("status_code")
                .and_then(Value::as_u64)
                .and_then(|status| u16::try_from(status).ok())
                .unwrap_or(400),
            headers: None,
            body: Some(message.clone()),
        }),
        Some(body) => match masked_text(body, key) {
            Some(texts) => Verdict::Mask(texts),
            // A mask that carried nothing readable is not a pass: it asked for
            // a rewrite and did not say to what.
            None => {
                tracing::warn!("a guard webhook masked with a body this build cannot read");
                Verdict::Reject(Rejection {
                    status: 503,
                    headers: None,
                    body: None,
                })
            }
        },
        None => Verdict::Pass,
    }
}

/// The replacement text in a mask action, in order.
fn masked_text(body: &Value, key: &str) -> Option<Vec<String>> {
    let items = body.get(key)?.as_array()?;
    let texts: Vec<String> = items
        .iter()
        .filter_map(|item| {
            // A prompt message carries `content`; a response choice wraps one
            // in `message`. Upstream uses one action shape for both.
            let message = item.get("message").unwrap_or(item);
            message.get("content")?.as_str().map(str::to_string)
        })
        .collect();
    (!texts.is_empty()).then_some(texts)
}

/// What a header expression may read.
///
/// The *client's* request, not the webhook request being built: `request.*`
/// and `jwt.*` mean what the caller sent. Snapshotted before the body is
/// consumed, since that is the only moment both exist.
#[derive(Debug, Default)]
pub struct Context {
    headers: BTreeMap<String, String>,
    claims: Option<Value>,
    llm_request: Option<Value>,
}

impl Context {
    /// Capture what expressions may read.
    pub fn new(headers: BTreeMap<String, String>, claims: Option<Value>) -> Self {
        Context {
            headers,
            claims,
            llm_request: None,
        }
    }

    /// Add the translated request, which `llmRequest.*` reads.
    pub fn with_llm_request(mut self, body: Value) -> Self {
        self.llm_request = Some(body);
        self
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// Evaluate one expression to a header value.
    ///
    /// `None` when it fails or produces nothing a header can carry. A guard
    /// header is context for a decision, not the decision, so one that cannot
    /// be computed is dropped rather than failing the call.
    fn eval(&self, source: &str) -> Option<String> {
        let program = cel::Program::compile(source).ok()?;
        let mut context = cel::Context::default();
        context
            .add_variable("request", json!({"headers": self.headers.clone()}))
            .ok()?;
        context
            .add_variable("jwt", self.claims.clone().unwrap_or(Value::Null))
            .ok()?;
        context
            .add_variable(
                "llmRequest",
                self.llm_request.clone().unwrap_or(Value::Null),
            )
            .ok()?;

        match program.execute(&context).ok()? {
            cel::Value::String(text) => Some(text.to_string()),
            cel::Value::Int(number) => Some(number.to_string()),
            cel::Value::UInt(number) => Some(number.to_string()),
            cel::Value::Bool(value) => Some(value.to_string()),
            cel::Value::Null => None,
            other => Some(format!("{other:?}")),
        }
    }
}

#[cfg(test)]
mod tests;
