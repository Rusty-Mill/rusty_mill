//! Route header modifiers on the upstream MCP request, and the guardrail
//! values they can read.
//!
//! `requestHeaderModifier` was consumed only by the `host` proxy path. On a
//! route with an `mcp` backend it parsed and did nothing — the failure this
//! project treats as worse than not supporting a field at all. It applies to
//! MCP upstream requests now, and its values can reference what a guardrail
//! decided:
//!
//! ```yaml
//! policies:
//!   mcpGuardrails: { processors: [...] }   # sets `classification`
//!   requestHeaderModifier:
//!     set:
//!       x-classification: "{{mcpGuardrails.classification}}"
//! ```
//!
//! That is upstream's `transformation` consumer for the metadata bag, in the
//! shape this gateway already has. A processor classifies a call in-band and
//! the classification reaches the MCP server behind the gateway, which can act
//! on it without speaking to the policy service itself.
//!
//! # Placeholders, not expressions
//!
//! `{{...}}` rather than bare CEL, because a header value is a string and
//! most of them are literals. Requiring a delimiter means adding this cannot
//! change what an existing static value means. Only `mcpGuardrails.<key>`
//! resolves; nothing else is in scope, and there is deliberately no way to
//! reach the request or the token from here — `metadata` expressions already
//! do that, on the way *out*.
//!
//! # An unresolved placeholder drops its header
//!
//! Rather than sending `{{mcpGuardrails.classification}}` upstream as though
//! it were data. A guardrail that did not run, or did not set that key, should
//! read as "no classification", and an absent header says that where a literal
//! template string says something false and confusing.
//!
//! # Only `mcp:` targets
//!
//! Same as `headerMutation`: a `stdio` target speaks over a pipe and has no
//! headers. The modifier is dropped there rather than quietly going nowhere.

use agentgateway_config::HeaderModifier;
use http::{HeaderName, HeaderValue};

use crate::{guardrails::Annotations, mutating_client::HeaderOverride};

/// A header modifier value that could not be turned into a header.
#[derive(Debug, thiserror::Error)]
#[error("{at}: `{value}` is not a valid {kind}")]
pub struct TransformError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The offending text.
    pub value: String,
    /// What it was being read as.
    pub kind: &'static str,
}

/// One header value, split into the literals and holes that make it up.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Template {
    /// No placeholders; the value is used as written.
    Literal(HeaderValue),
    /// Alternating literal text and annotation keys to fill in.
    Parts(Vec<Part>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    /// A `mcpGuardrails.<key>` reference, by key.
    Annotation(String),
}

/// The prefix a placeholder must carry to resolve.
const NAMESPACE: &str = "mcpGuardrails.";

impl Template {
    /// Parse one configured value.
    ///
    /// A value with no `{{` is kept as a pre-validated [`HeaderValue`], so the
    /// common case costs nothing per request and a bad literal is caught at
    /// startup rather than on the first call.
    fn parse(value: &str, at: &str) -> Result<Self, TransformError> {
        if !value.contains("{{") {
            let value = HeaderValue::try_from(value).map_err(|_| TransformError {
                at: at.to_string(),
                value: value.to_string(),
                kind: "header value",
            })?;
            return Ok(Template::Literal(value));
        }

        let mut parts = Vec::new();
        let mut rest = value;

        while let Some(open) = rest.find("{{") {
            if !rest[..open].is_empty() {
                parts.push(Part::Text(rest[..open].to_string()));
            }
            let after = &rest[open + 2..];
            let Some(close) = after.find("}}") else {
                return Err(TransformError {
                    at: at.to_string(),
                    value: value.to_string(),
                    kind: "header value: an unclosed `{{`",
                });
            };

            let reference = after[..close].trim();
            let Some(key) = reference.strip_prefix(NAMESPACE) else {
                return Err(TransformError {
                    at: at.to_string(),
                    value: reference.to_string(),
                    kind: "placeholder: only `mcpGuardrails.<key>` resolves here",
                });
            };
            if key.is_empty() {
                return Err(TransformError {
                    at: at.to_string(),
                    value: reference.to_string(),
                    kind: "placeholder: a key is missing after `mcpGuardrails.`",
                });
            }
            parts.push(Part::Annotation(key.to_string()));
            rest = &after[close + 2..];
        }

        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_string()));
        }

        Ok(Template::Parts(parts))
    }

    /// Fill this template in, or `None` when a placeholder has no value.
    fn resolve(&self, annotations: &Annotations) -> Option<HeaderValue> {
        match self {
            Template::Literal(value) => Some(value.clone()),
            Template::Parts(parts) => {
                let mut out = String::new();
                for part in parts {
                    match part {
                        Part::Text(text) => out.push_str(text),
                        Part::Annotation(key) => {
                            let value = annotations.iter().find(|(name, _)| *name == key)?.1;
                            match value {
                                // A string renders as itself rather than as a
                                // quoted JSON string.
                                serde_json::Value::String(text) => out.push_str(text),
                                serde_json::Value::Null => return None,
                                other => out.push_str(&other.to_string()),
                            }
                        }
                    }
                }
                HeaderValue::try_from(out).ok()
            }
        }
    }
}

/// A route's `requestHeaderModifier`, compiled for the MCP upstream path.
#[derive(Debug, Clone, Default)]
pub struct Transform {
    set: Vec<(HeaderName, Template)>,
    /// `add` is folded into `set` for the MCP path; see [`Transform::apply`].
    add: Vec<(HeaderName, Template)>,
    remove: Vec<HeaderName>,
}

impl Transform {
    /// Compile a route's modifier, reporting the first thing HTTP rejects.
    pub fn new(modifier: &HeaderModifier, at: &str) -> Result<Self, TransformError> {
        let name = |raw: &String| -> Result<HeaderName, TransformError> {
            HeaderName::try_from(raw.as_str()).map_err(|_| TransformError {
                at: at.to_string(),
                value: raw.clone(),
                kind: "header name",
            })
        };

        let mut set = Vec::with_capacity(modifier.set.len());
        for (key, value) in &modifier.set {
            set.push((name(key)?, Template::parse(value, at)?));
        }
        let mut add = Vec::with_capacity(modifier.add.len());
        for (key, value) in &modifier.add {
            add.push((name(key)?, Template::parse(value, at)?));
        }
        let mut remove = Vec::with_capacity(modifier.remove.len());
        for key in &modifier.remove {
            remove.push(name(key)?);
        }

        Ok(Transform { set, add, remove })
    }

    /// Whether the route configured anything.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.add.is_empty() && self.remove.is_empty()
    }

    /// Fold this modifier into the changes already destined for the upstream.
    ///
    /// Runs *after* a guardrail's `headerMutation`, so route configuration
    /// wins over a processor's runtime decision — the operator's intent is the
    /// one written down. That is also upstream's ordering: the metadata bag
    /// exists so that "subsequent backend filters" can read it.
    ///
    /// `add` cannot append here the way it does on the HTTP proxy path. What
    /// crosses to the transport is one value per name, so `add` behaves as
    /// `set` unless the same name is already spoken for, in which case the
    /// values are joined into one comma-separated field line — which is how
    /// HTTP spells a list in a single header.
    pub fn apply(&self, changes: &mut HeaderOverride, annotations: &Annotations) {
        for (name, template) in &self.set {
            let Some(value) = template.resolve(annotations) else {
                unresolved(name);
                continue;
            };
            changes.remove.retain(|dropped| *dropped != name);
            match changes
                .set
                .iter_mut()
                .find(|(existing, _)| existing == name)
            {
                Some((_, slot)) => *slot = value,
                None => changes.set.push((name.clone(), value)),
            }
        }

        for (name, template) in &self.add {
            let Some(value) = template.resolve(annotations) else {
                unresolved(name);
                continue;
            };
            changes.remove.retain(|dropped| *dropped != name);
            match changes
                .set
                .iter_mut()
                .find(|(existing, _)| existing == name)
            {
                Some((_, slot)) => *slot = join(slot, &value),
                None => changes.set.push((name.clone(), value)),
            }
        }

        for name in &self.remove {
            changes.set.retain(|(existing, _)| existing != name);
            if !changes.remove.contains(name) {
                changes.remove.push(name.clone());
            }
        }
    }
}

fn unresolved(name: &HeaderName) {
    tracing::debug!(
        header = %name,
        "a header template referenced a guardrail value that is not set; dropping the header"
    );
}

/// Join two header values into one comma-separated field line.
fn join(first: &HeaderValue, second: &HeaderValue) -> HeaderValue {
    let mut bytes = first.as_bytes().to_vec();
    bytes.extend_from_slice(b", ");
    bytes.extend_from_slice(second.as_bytes());
    HeaderValue::from_bytes(&bytes).unwrap_or_else(|_| second.clone())
}

#[cfg(test)]
mod tests;
