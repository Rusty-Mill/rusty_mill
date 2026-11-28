//! A target's `identity`: who the gateway acts for, stamped from the token.
//!
//! The gateway reaches each upstream with its own credential, so the upstream
//! learns whom a call is for only from what the gateway says. This module says
//! it, and only from the verified token's claims (Let Em Loose D101, D223,
//! 726-sandbox-design §3):
//!
//! 1. `strip` removes the arguments an agent could use to name itself
//!    (`actor`, `agent_id`, ...), per tool-name glob.
//! 2. `set` writes arguments from claim templates, overwriting anything the
//!    caller sent under the same name.
//! 3. `headers` adds headers from claim templates (`mcp:` targets only).
//!
//! It runs last, after the guardrail chain, so nothing a processor rewrites can
//! put an agent-chosen identity back.
//!
//! # Fails closed
//!
//! A template whose claim is missing, empty or not a string or number refuses
//! the call. Sending the call without the identity would let the upstream fall
//! back to whatever default it has, which is the gap this exists to close.

use std::collections::BTreeMap;

use agentgateway_config::TargetIdentity;
use http::{HeaderName, HeaderValue};
use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::CallToolParams;

use crate::glob::glob;
use crate::header_override::HeaderOverride;

/// The prefix a placeholder must carry: only token claims resolve.
const NAMESPACE: &str = "jwt.";

/// A target's `identity` that is wrong, or a call it can't stamp.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    /// The configuration did not compile.
    #[error("{at}: `{value}` is not a valid {kind}")]
    Config {
        /// Where in the configuration it came from.
        at: String,
        /// The offending text.
        value: String,
        /// What it was being read as.
        kind: &'static str,
    },

    /// A template names a claim the token does not carry as text.
    #[error("the token carries no usable `{claim}` claim, so the call can't say whom it is for")]
    Unresolved {
        /// The claim key.
        claim: String,
    },

    /// A claim rendered into a header is not visible ASCII.
    #[error("the token's claims can't be sent as header `{name}` (visible ASCII only)")]
    Header {
        /// The header name.
        name: String,
    },

    /// The call's arguments are not a JSON object, so nothing can be set.
    #[error("tool arguments must be a JSON object")]
    Arguments,
}

/// One piece of a claim template.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    /// A claim key, and how many leading characters to keep.
    Claim {
        key: String,
        take: Option<usize>,
    },
}

/// A value written as literal text and `{{jwt.<claim>[:n]}}` placeholders.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Template(Vec<Part>);

impl Template {
    fn parse(value: &str, at: &str) -> Result<Self, IdentityError> {
        let bad = |value: &str, kind| IdentityError::Config {
            at: at.to_owned(),
            value: value.to_owned(),
            kind,
        };
        let mut parts = Vec::new();
        let mut rest = value;
        while let Some(open) = rest.find("{{") {
            if open > 0 {
                parts.push(Part::Text(rest[..open].to_owned()));
            }
            let after = &rest[open + 2..];
            let close = after
                .find("}}")
                .ok_or_else(|| bad(value, "template: an unclosed `{{`"))?;
            parts.push(Self::claim(after[..close].trim()).ok_or_else(|| {
                bad(
                    &after[..close],
                    "placeholder: write `jwt.<claim>` or `jwt.<claim>:<n>`",
                )
            })?);
            rest = &after[close + 2..];
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_owned()));
        }
        if !parts.iter().any(|p| matches!(p, Part::Claim { .. })) {
            // An identity that names no claim is a constant, and a constant
            // identity says nothing about who called.
            return Err(bad(value, "template: it names no `{{jwt.<claim>}}`"));
        }
        Ok(Template(parts))
    }

    fn claim(reference: &str) -> Option<Part> {
        let reference = reference.strip_prefix(NAMESPACE)?;
        let (key, take) = match reference.rsplit_once(':') {
            Some((key, n)) => (key, Some(n.parse::<usize>().ok().filter(|n| *n > 0)?)),
            None => (reference, None),
        };
        (!key.is_empty()).then(|| Part::Claim {
            key: key.to_owned(),
            take,
        })
    }

    fn resolve(&self, claims: Option<&serde_json::Value>) -> Result<String, IdentityError> {
        let mut out = String::new();
        for part in &self.0 {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Claim { key, take } => {
                    let value = claim_text(claims, key)
                        .ok_or_else(|| IdentityError::Unresolved { claim: key.clone() })?;
                    match take {
                        Some(n) => out.extend(value.chars().take(*n)),
                        None => out.push_str(&value),
                    }
                }
            }
        }
        Ok(out)
    }
}

/// A claim as text: a non-empty string, or a number. Keys are top-level and
/// literal, so `lel.lens` is the key `"lel.lens"`, as the token writes it.
fn claim_text(claims: Option<&serde_json::Value>, key: &str) -> Option<String> {
    match claims?.get(key)? {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A target's `identity`, compiled.
#[derive(Debug, Clone, Default)]
pub struct Identity {
    strip: Vec<(String, Vec<String>)>,
    set: Vec<(String, Template)>,
    headers: Vec<(HeaderName, Template)>,
}

impl Identity {
    /// Compile a target's `identity`, reporting the first bad template or name.
    ///
    /// # Errors
    /// [`IdentityError::Config`] naming where it is.
    pub fn new(config: &TargetIdentity, at: &str) -> Result<Self, IdentityError> {
        let strip = config
            .strip
            .iter()
            .map(|(tools, args)| (tools.clone(), args.clone()))
            .collect();
        let set = compile(&config.set, &format!("{at}.identity.set"), |name, _| {
            Ok(name.to_owned())
        })?;
        let headers = compile(
            &config.headers,
            &format!("{at}.identity.headers"),
            |name, at| {
                HeaderName::try_from(name).map_err(|_| IdentityError::Config {
                    at: at.to_owned(),
                    value: name.to_owned(),
                    kind: "header name",
                })
            },
        )?;
        Ok(Identity {
            strip,
            set,
            headers,
        })
    }

    /// Whether the target configured anything.
    pub fn is_empty(&self) -> bool {
        self.strip.is_empty() && self.set.is_empty() && self.headers.is_empty()
    }

    /// Whether it sends headers, which only an `mcp:` target can carry.
    pub fn has_headers(&self) -> bool {
        !self.headers.is_empty()
    }

    /// Stamp one call with the caller's identity: strip, then set, then add
    /// headers. `strip` matches `params.name`, the name the upstream will
    /// receive, so call this after every rewrite. Nothing is changed unless
    /// every template resolves.
    ///
    /// # Errors
    /// [`IdentityError::Unresolved`], [`IdentityError::Header`] or
    /// [`IdentityError::Arguments`]; the call must then be refused.
    pub fn apply(
        &self,
        params: &mut CallToolParams,
        headers: &mut HeaderOverride,
        claims: Option<&serde_json::Value>,
    ) -> Result<(), IdentityError> {
        let set = self
            .set
            .iter()
            .map(|(arg, t)| Ok((arg, t.resolve(claims)?)))
            .collect::<Result<Vec<_>, IdentityError>>()?;
        let stamped = self
            .headers
            .iter()
            .map(|(name, t)| {
                let value = t.resolve(claims)?;
                // Only visible ASCII survives every transport (the native
                // client drops a value `to_str` refuses), so anything else is
                // refused rather than sent without the header.
                let value = HeaderValue::try_from(value)
                    .ok()
                    .filter(|v| v.to_str().is_ok())
                    .ok_or_else(|| IdentityError::Header {
                        name: name.as_str().to_owned(),
                    })?;
                Ok((name, value))
            })
            .collect::<Result<Vec<_>, IdentityError>>()?;

        let tool = params.name.clone();
        self.stamp_arguments(&tool, params, set)?;
        for (name, value) in stamped {
            headers.remove.retain(|dropped| dropped != name);
            headers.set.retain(|(existing, _)| existing != name);
            headers.set.push((name.clone(), value));
        }
        Ok(())
    }

    fn stamp_arguments(
        &self,
        tool: &str,
        params: &mut CallToolParams,
        set: Vec<(&String, String)>,
    ) -> Result<(), IdentityError> {
        let strip: Vec<&String> = self
            .strip
            .iter()
            .filter(|(pattern, _)| glob(pattern, tool))
            .flat_map(|(_, args)| args)
            .collect();
        if strip.is_empty() && set.is_empty() {
            return Ok(());
        }
        let arguments = params.arguments.get_or_insert_with(Value::object);
        if arguments.is_null() {
            *arguments = Value::object();
        }
        let map = arguments.as_object_mut().ok_or(IdentityError::Arguments)?;
        for arg in strip {
            map.remove(arg.as_str());
        }
        for (arg, value) in set {
            map.insert(arg.clone(), Value::String(value));
        }
        Ok(())
    }
}

/// Compile a name-to-template map, naming each entry's place on failure.
fn compile<K>(
    entries: &BTreeMap<String, String>,
    at: &str,
    key: impl Fn(&str, &str) -> Result<K, IdentityError>,
) -> Result<Vec<(K, Template)>, IdentityError> {
    entries
        .iter()
        .map(|(name, value)| {
            let place = format!("{at}.{name}");
            Ok((key(name, &place)?, Template::parse(value, &place)?))
        })
        .collect()
}

#[cfg(test)]
mod tests;
