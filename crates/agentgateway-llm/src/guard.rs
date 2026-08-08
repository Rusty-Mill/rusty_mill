//! `promptGuard.regex`: content rules over a prompt and over what comes back.
//!
//! A rule either **rejects** — the request stops here and the operator's own
//! body goes back — or **masks**, replacing what matched and carrying on.
//!
//! # Where the text is
//!
//! On the request, the text is every `messages[].content` string, and masking
//! rewrites them in place. On the response it is the assistant's own text.
//! Both run on the OpenAI shape, before translation and after it, so a rule
//! written once means the same thing for either provider.
//!
//! A tool call's arguments are deliberately **not** scanned. They are a
//! structured object the model produced from a schema the operator wrote, not
//! free text a person typed, and masking inside one produces JSON that no
//! longer matches the schema the tool will be called with.
//!
//! # A response rule buffers a streamed answer
//!
//! This is the part worth being explicit about, because it costs something a
//! caller will notice.
//!
//! A pattern can straddle a chunk boundary: `"my number is 555-"` arrives, then
//! `"1234"`. Scanning each chunk on its own misses that, and by the time the
//! second chunk shows what the first one started, the first is already at the
//! client and cannot be recalled.
//!
//! A sliding window — hold back the last N bytes, scan across the join — keeps
//! the stream but has to pick N, and a regex has no general longest-match
//! bound: `\d+` can run past any window. The failure would be a silent leak of
//! exactly the thing the rule exists to catch, which is worse than the failure
//! being obvious.
//!
//! So a route with a response rule collects the whole answer, applies the rule
//! and then sends it — as one content chunk, since the text is no longer the
//! text the provider chunked. The cost is real and the operator chose it by
//! asking for the rule; `--check` and the startup log both say so, rather than
//! leaving someone to notice their stream became one lump.

pub mod webhook;

use agentgateway_config::{Builtin, GuardAction, GuardPattern, GuardRule, PromptGuard, RegexGuard};
use regex::RegexSet;
use serde_json::Value;

/// A pattern that would not compile.
#[derive(Debug, thiserror::Error)]
#[error("{at}: `{pattern}` is not a valid regular expression: {reason}")]
pub struct GuardError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The offending pattern.
    pub pattern: String,
    /// What the engine said.
    pub reason: String,
}

/// The text a builtin matches, and the token it is replaced with.
///
/// Deliberately conservative. A pattern that matches too much on a `mask` rule
/// silently eats an answer, and one that matches too much on `reject` refuses
/// traffic nobody meant to refuse — both read as the gateway being broken.
fn builtin(kind: Builtin) -> (&'static str, &'static str) {
    match kind {
        Builtin::Email => (
            r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}",
            "<EMAIL>",
        ),
        // Optional country code, then ten digits in the usual groupings.
        Builtin::PhoneNumber => (
            r"(?:\+?\d{1,3}[ \-.]?)?\(?\d{3}\)?[ \-.]?\d{3}[ \-.]?\d{4}\b",
            "<PHONE_NUMBER>",
        ),
        Builtin::Ssn => (r"\b\d{3}-\d{2}-\d{4}\b", "<SSN>"),
        // Thirteen to sixteen digits, optionally grouped. Not Luhn-checked:
        // this is a redaction rule, and a number that looks like a card is
        // worth masking whether or not it would validate.
        Builtin::CreditCard => (r"\b(?:\d{4}[ \-]?){3}\d{1,4}\b", "<CREDIT_CARD>"),
        Builtin::CaSin => (r"\b\d{3}[ \-]?\d{3}[ \-]?\d{3}\b", "<CA_SIN>"),
    }
}

/// What a custom pattern is replaced with.
///
/// A builtin says what it found; an operator's own pattern could be anything,
/// so there is nothing more specific to say than that something was removed.
const CUSTOM_MASK: &str = "<masked>";

/// One compiled rule.
///
/// A rule is either patterns or a webhook. Upstream's schema allows both keys
/// on one rule, but nothing sensible follows from a pattern *and* a service
/// disagreeing, so they are kept separate and run in the order written.
#[derive(Debug)]
enum Rule {
    /// Patterns decided in advance.
    Patterns(Patterns),
    /// A service that can change its mind.
    Webhook(webhook::Webhook),
}

/// A compiled `regex` rule.
#[derive(Debug)]
struct Patterns {
    action: GuardAction,
    /// Compiled individually so a match can be replaced with the right token.
    patterns: Vec<(regex::Regex, &'static str)>,
    /// The same patterns as a set, which answers "did anything match" in one
    /// pass rather than one pass per pattern.
    set: RegexSet,
    rejection: Rejection,
}

/// The refusal a rule answers with.
#[derive(Debug, Clone)]
pub struct Rejection {
    /// Status to answer with.
    pub status: u16,
    /// Headers the operator wants on it.
    pub headers: Option<agentgateway_config::HeaderModifier>,
    /// The body, or `None` for this crate's own error envelope.
    pub body: Option<String>,
}

/// What a phase decided.
#[derive(Debug)]
pub enum Decision {
    /// Nothing matched; the text is unchanged.
    Allowed,
    /// Something matched a `mask` rule and the text was rewritten.
    Masked,
    /// A `reject` rule matched.
    Rejected(Rejection),
}

/// A route's compiled `promptGuard`.
#[derive(Debug)]
pub struct Guard {
    request: Vec<Rule>,
    response: Vec<Rule>,
}

impl Guard {
    /// Compile a policy, or `None` when it has no rule this build acts on.
    pub fn new(policy: Option<&PromptGuard>, at: &str) -> Result<Option<Self>, GuardError> {
        let Some(policy) = policy else {
            return Ok(None);
        };
        let request = compile(&policy.request, &format!("{at}.request"))?;
        let response = compile(&policy.response, &format!("{at}.response"))?;
        if request.is_empty() && response.is_empty() {
            return Ok(None);
        }
        Ok(Some(Guard { request, response }))
    }

    /// Whether a response rule exists, and so whether a stream is buffered.
    pub fn guards_response(&self) -> bool {
        !self.response.is_empty()
    }

    /// Whether any rule here talks to something outside the process.
    ///
    /// Only a webhook needs the caller's headers and claims snapshotted, and
    /// snapshotting costs an allocation per request — so a route of pure
    /// `regex` rules does not pay for a context nothing will read.
    pub fn calls_out(&self) -> bool {
        self.request
            .iter()
            .chain(self.response.iter())
            .any(|rule| matches!(rule, Rule::Webhook(_)))
    }

    /// Apply the request rules to an OpenAI-shaped body.
    ///
    /// Rules run in order and the first refusal ends it, so an operator can
    /// read a list top to bottom and know which refusal a request will get.
    /// A `webhook` rule is a network call, which is why this is async and why
    /// putting a cheap `regex` rule above an expensive one is worth doing.
    pub async fn check_request(&self, body: &mut Value, context: &webhook::Context) -> Decision {
        let mut decision = Decision::Allowed;
        for rule in &self.request {
            let outcome = match rule {
                Rule::Patterns(patterns) => {
                    let mut outcome = Decision::Allowed;
                    for text in message_texts(body) {
                        match patterns.apply(text) {
                            Decision::Rejected(rejection) => {
                                return Decision::Rejected(rejection);
                            }
                            Decision::Masked => outcome = Decision::Masked,
                            Decision::Allowed => {}
                        }
                    }
                    outcome
                }
                Rule::Webhook(webhook) => {
                    let messages = messages_for(body);
                    match webhook.check_messages(&messages, context).await {
                        webhook::Verdict::Pass => Decision::Allowed,
                        webhook::Verdict::Reject(rejection) => {
                            return Decision::Rejected(rejection);
                        }
                        webhook::Verdict::Mask(replacements) => {
                            // Positional, as upstream's shape is: the webhook
                            // sends back the same list it was given.
                            for (text, replacement) in
                                message_texts(body).zip(replacements.into_iter())
                            {
                                *text = replacement;
                            }
                            Decision::Masked
                        }
                    }
                }
            };
            if matches!(outcome, Decision::Masked) {
                decision = Decision::Masked;
            }
        }
        decision
    }

    /// Apply the response rules to the assistant's own text.
    pub async fn check_text(&self, text: &mut String, context: &webhook::Context) -> Decision {
        let mut decision = Decision::Allowed;
        for rule in &self.response {
            let outcome = match rule {
                Rule::Patterns(patterns) => patterns.apply(text),
                Rule::Webhook(webhook) => match webhook.check_answer(text, context).await {
                    webhook::Verdict::Pass => Decision::Allowed,
                    webhook::Verdict::Reject(rejection) => return Decision::Rejected(rejection),
                    webhook::Verdict::Mask(replacements) => {
                        if let Some(replacement) = replacements.into_iter().next() {
                            *text = replacement;
                        }
                        Decision::Masked
                    }
                },
            };
            match outcome {
                Decision::Rejected(rejection) => return Decision::Rejected(rejection),
                Decision::Masked => decision = Decision::Masked,
                Decision::Allowed => {}
            }
        }
        decision
    }
}

/// The conversation as a webhook is shown it.
fn messages_for(body: &Value) -> Vec<(String, String)> {
    body.get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .filter_map(|message| {
                    let role = message.get("role")?.as_str().unwrap_or("user").to_string();
                    let content = message.get("content")?.as_str()?.to_string();
                    Some((role, content))
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Patterns {
    /// Scan one piece of text, rewriting it when the rule masks.
    fn apply(&self, text: &mut String) -> Decision {
        if !self.set.is_match(text) {
            return Decision::Allowed;
        }
        match self.action {
            GuardAction::Reject => Decision::Rejected(self.rejection.clone()),
            GuardAction::Mask => {
                for (pattern, token) in &self.patterns {
                    // `Cow` so a pattern that did not match costs no
                    // allocation -- most of them will not, on most calls.
                    if let std::borrow::Cow::Owned(replaced) = pattern.replace_all(text, *token) {
                        *text = replaced;
                    }
                }
                Decision::Masked
            }
        }
    }
}

/// Every message content string in an OpenAI request.
///
/// A structured content list is the multimodal shape; its text parts are
/// reachable but its image parts are not text at all, so this build scans the
/// plain form and leaves the other alone rather than half doing it.
fn message_texts(body: &mut Value) -> impl Iterator<Item = &mut String> {
    body.get_mut("messages")
        .and_then(Value::as_array_mut)
        .map(|messages| messages.as_mut_slice())
        .unwrap_or_default()
        .iter_mut()
        .filter_map(|message| match message.get_mut("content") {
            Some(Value::String(text)) => Some(text),
            _ => None,
        })
}

fn compile(rules: &[GuardRule], at: &str) -> Result<Vec<Rule>, GuardError> {
    let mut compiled = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        if let Some(regex) = rule.regex.as_ref() {
            compiled.push(Rule::Patterns(compile_one(
                regex,
                rule.rejection.as_ref(),
                &format!("{at}[{i}].regex"),
            )?));
        }
        if let Some(config) = rule.webhook.as_ref() {
            compiled.push(Rule::Webhook(webhook::Webhook::new(config)));
        }
        // Anything else is a rule kind `Config::lint` reports.
    }
    Ok(compiled)
}

fn compile_one(
    regex: &RegexGuard,
    rejection: Option<&agentgateway_config::Rejection>,
    at: &str,
) -> Result<Patterns, GuardError> {
    let mut patterns = Vec::with_capacity(regex.rules.len());
    let mut sources = Vec::with_capacity(regex.rules.len());

    for pattern in &regex.rules {
        let (source, token) = match pattern {
            GuardPattern::Pattern(source) => (source.clone(), CUSTOM_MASK),
            GuardPattern::Builtin(kind) => {
                let (source, token) = builtin(*kind);
                (source.to_string(), token)
            }
        };
        // A pattern that does not compile is a startup failure. The
        // alternative is a rule that silently never fires, which reads exactly
        // like content nobody sent.
        let compiled = regex::Regex::new(&source).map_err(|err| GuardError {
            at: at.to_string(),
            pattern: source.clone(),
            reason: err.to_string(),
        })?;
        patterns.push((compiled, token));
        sources.push(source);
    }

    let set = RegexSet::new(&sources).map_err(|err| GuardError {
        at: at.to_string(),
        pattern: sources.join(", "),
        reason: err.to_string(),
    })?;

    let rejection = rejection.cloned().unwrap_or_default();
    Ok(Patterns {
        action: regex.action,
        patterns,
        set,
        rejection: Rejection {
            status: rejection.status,
            headers: rejection.headers,
            body: rejection.body,
        },
    })
}

#[cfg(test)]
mod tests;
