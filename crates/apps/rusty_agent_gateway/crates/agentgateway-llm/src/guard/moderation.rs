//! `promptGuard.openAIModeration`: OpenAI's classifier asked about a prompt.
//!
//! Where [`super`]'s `regex` rules decide from a pattern written down in
//! advance and a `webhook` asks a service the operator runs, this asks a model
//! trained to recognise categories — harassment, self-harm, sexual content,
//! violence — that no pattern describes.
//!
//! The endpoint answers with a verdict per input and a `flagged` flag on each.
//! Upstream refuses when anything is flagged and so does this, without looking
//! at which category: a rule that fired is a rule that fired, and picking
//! categories apart would be inventing a policy language upstream does not
//! have.
//!
//! # It classifies the prompt, never the answer
//!
//! Upstream's response guard has no moderation variant, and this follows it.
//! `Config::lint` reports a rule written under `response`.
//!
//! # Where the credential comes from
//!
//! A moderation call is an authenticated call to OpenAI, so it needs an OpenAI
//! key. Two ways to have one, and a deliberate refusal in between:
//!
//! - the rule's own `policies.backendAuth.key`, which goes to
//!   `api.openai.com`; or
//! - the route's `backendAuth.key`, borrowed **only when the route's provider
//!   is `openAI`**.
//!
//! The second is the case worth being careful about. A route on Anthropic has
//! a key too, and borrowing it would send an Anthropic credential to OpenAI —
//! a secret handed to a third party who was never meant to see it, by a
//! gateway the operator trusted to do the opposite. So a moderation rule on a
//! non-OpenAI route without its own key does not start.
//!
//! A borrowed key travels only as far as the route it was borrowed from: the
//! call goes to that route's own base URL, `hostOverride` included, rather
//! than to `api.openai.com`. This is a narrow divergence from upstream, which
//! always calls `api.openai.com`, and it exists for the same reason as the
//! refusal above — a key issued for one host should not be sent to another.
//! An operator who wants the real OpenAI from a route pointed elsewhere says
//! so by giving the rule its own key.
//!
//! # It fails closed
//!
//! An endpoint that cannot be reached, times out, or answers with something
//! unreadable **refuses** the request. Upstream's moderation carries no
//! `failureMode` — unlike a webhook, which can be told to fail open — so there
//! is nothing to configure and no fail-open path here. A content control that
//! waves traffic through when its classifier is down is not one.

use std::time::Duration;

use agentgateway_config::BackendAuth;
use serde_json::{Value, json};

use super::{Decision, Rejection};

/// The classifier upstream asks for when a rule names none.
const DEFAULT_MODEL: &str = "omni-moderation-latest";

/// Where the moderation endpoint lives, relative to an OpenAI base URL.
const PATH: &str = "/v1/moderations";

/// OpenAI itself, for a rule carrying its own key.
const OPENAI: &str = "https://api.openai.com";

/// Budget for one moderation call.
///
/// This sits in front of every request on the route, so a slow classifier is a
/// slow gateway. The same five seconds a guard webhook gets, for the same
/// reason: it is running a model rather than a lookup, and a hung one should
/// be noticed rather than waited on.
const TIMEOUT: Duration = Duration::from_secs(5);

/// A moderation rule that cannot be called.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    /// The rule named `passthrough`.
    #[error(
        "{at}: `passthrough` would send the caller's own bearer token to OpenAI's moderation \
         endpoint, which is not what a client's token is; name a `key`"
    )]
    Passthrough {
        /// Where in the configuration it came from.
        at: String,
    },
    /// Nothing on the route or the rule is an OpenAI key.
    #[error(
        "{at}: moderation calls OpenAI and has no key to call it with; give the rule a \
         `policies.backendAuth.key`{}",
        match provider {
            Some(provider) =>
                format!(", because a `{provider}` route's own key is not an OpenAI credential \
                         and sending it to OpenAI would hand a secret to a third party"),
            None => ", or give the route a `backendAuth.key` to borrow".to_string(),
        }
    )]
    Missing {
        /// Where in the configuration it came from.
        at: String,
        /// The route's provider, when it is one whose key cannot be borrowed.
        provider: Option<&'static str>,
    },
}

/// What a moderation rule may borrow from the route it sits on.
///
/// Built only for a route whose provider is `openAI`; see the module docs for
/// why a route on anything else lends nothing.
#[derive(Debug, Clone, Copy)]
pub struct Borrowable<'a> {
    /// The route's own moderation URL, `hostOverride` included.
    pub endpoint: &'a str,
    /// The route's `backendAuth.key`, if it has one.
    pub key: Option<&'a str>,
}

/// A compiled `openAIModeration` rule.
#[derive(Debug)]
pub struct Moderation {
    endpoint: String,
    model: String,
    key: String,
    rejection: Rejection,
    client: reqwest::Client,
}

impl Moderation {
    /// Compile one rule, resolving the credential it will call with.
    pub fn new(
        config: &agentgateway_config::Moderation,
        rejection: Rejection,
        borrowable: Option<Borrowable<'_>>,
        route_provider: &'static str,
        at: &str,
    ) -> Result<Self, CredentialError> {
        let own = config
            .policies
            .as_ref()
            .and_then(|policies| policies.backend_auth.as_ref());

        let (endpoint, key) = match own {
            Some(BackendAuth::Key(key)) => (format!("{OPENAI}{PATH}"), key.clone()),
            Some(BackendAuth::Passthrough(_)) => {
                return Err(CredentialError::Passthrough { at: at.to_string() });
            }
            None => match borrowable {
                Some(Borrowable {
                    endpoint,
                    key: Some(key),
                }) => (endpoint.to_string(), key.to_string()),
                // An OpenAI route with no key of its own: there is nothing to
                // borrow, so the rule needs one. Named separately from the
                // provider case because the fix is different.
                Some(Borrowable { key: None, .. }) => {
                    return Err(CredentialError::Missing {
                        at: at.to_string(),
                        provider: None,
                    });
                }
                None => {
                    return Err(CredentialError::Missing {
                        at: at.to_string(),
                        provider: Some(route_provider),
                    });
                }
            },
        };

        Ok(Moderation {
            endpoint,
            model: config
                .model
                .clone()
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            key,
            rejection,
            client: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .unwrap_or_default(),
        })
    }

    /// Ask about a prompt.
    ///
    /// Nothing to classify is not a refusal: a request whose messages carry no
    /// text at all — an image-only turn, say — has sent nothing this endpoint
    /// reads, and calling it with an empty list only produces a 400 that would
    /// refuse the route's whole traffic.
    pub async fn check(&self, inputs: Vec<String>) -> Decision {
        if inputs.is_empty() {
            tracing::debug!("a moderation rule found no text in the prompt to classify");
            return Decision::Allowed;
        }

        match self.ask(inputs).await {
            Ok(flagged) => match flagged {
                true => Decision::Rejected(self.rejection.clone()),
                false => Decision::Allowed,
            },
            Err(reason) => {
                tracing::warn!(%reason, "the moderation endpoint could not be consulted");
                Decision::Rejected(unreachable())
            }
        }
    }

    async fn ask(&self, inputs: Vec<String>) -> Result<bool, String> {
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.key)
            .json(&json!({"input": inputs, "model": self.model}))
            .send()
            .await
            .map_err(|err| err.to_string())?;

        if !response.status().is_success() {
            // An endpoint that answers 401 or 500 has classified nothing, so
            // this is the unreachable case rather than a verdict it reached.
            return Err(format!("it answered {}", response.status()));
        }

        let answered: Value = response.json().await.map_err(|err| err.to_string())?;
        read_flagged(&answered).ok_or_else(|| "it answered without any `results`".to_string())
    }
}

/// Whether any result in a moderation response is flagged.
///
/// `None` when the body carries no results at all, which is not a verdict and
/// so is not a pass.
fn read_flagged(answered: &Value) -> Option<bool> {
    let results = answered.get("results")?.as_array()?;
    if results.is_empty() {
        return None;
    }
    Some(
        results
            .iter()
            .any(|result| result.get("flagged").and_then(Value::as_bool) == Some(true)),
    )
}

/// The refusal a classifier that could not be reached produces.
///
/// 503, not the rule's own rejection: nothing decided this prompt was
/// unacceptable, and answering with a content refusal would send someone to
/// inspect a prompt that is fine when the real problem is a service being
/// down.
fn unreachable() -> Rejection {
    Rejection {
        status: 503,
        headers: None,
        body: None,
    }
}

/// The text a moderation call is given, in message order.
///
/// Both the plain `content: "..."` form and the text parts of the multimodal
/// list form, which is more than [`super::message_texts`] reads. The reason
/// they differ is that this one only *reads*: a `regex` rule masks by
/// rewriting what it matched, and rewriting inside a content part means
/// rebuilding a structure whose other parts are images. Classifying text
/// wherever it is found carries no such risk, and skipping it would let a
/// prompt evade the rule by spelling itself the other way.
pub fn inputs(body: &Value) -> Vec<String> {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut inputs = Vec::new();
    for message in messages {
        match message.get("content") {
            Some(Value::String(text)) => inputs.push(text.clone()),
            Some(Value::Array(parts)) => {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        inputs.push(text.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    inputs
}

#[cfg(test)]
mod tests;
