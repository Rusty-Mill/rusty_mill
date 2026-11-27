//! Identity the gateway stamps on calls to one target (Let Em Loose #1376).
//!
//! The gateway authenticates to an upstream with its own credential and says
//! who it acts for. That statement must come from the caller's verified token,
//! never from the call: `strip` removes arguments a caller could use to pose as
//! someone else (Mill Board's `actor`, say), `set` writes them back from the
//! token, and `headers` carries the same to HTTP upstreams.
//!
//! # Templates
//!
//! `{{jwt.<claim>}}` names one top-level claim, whole: `{{jwt.lel.lens}}` reads
//! the claim called `lel.lens`, which is how the sandbox design names its
//! claims. Only string claims resolve.
//!
//! # Fails closed
//!
//! Unlike a route's header modifier, which drops a header it cannot fill, a
//! call whose identity cannot be stamped is refused. Sending it on unstamped
//! would let the upstream fall back to whatever the caller supplied, or to the
//! gateway's own name, and either is a false identity.

use std::collections::BTreeMap;

use agentgateway_config::McpIdentity;
use http::{HeaderName, HeaderValue};
use rusty_mcp_server::json::Value;

use crate::header_override::HeaderOverride;

/// The prefix a placeholder must carry.
const NAMESPACE: &str = "jwt.";

/// An identity block that could not be compiled.
#[derive(Debug, thiserror::Error)]
#[error("{at}: {problem}: `{value}`")]
pub struct IdentityError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The offending text.
    pub value: String,
    /// What is wrong with it.
    pub problem: &'static str,
}

/// Why one call could not be stamped.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum StampError {
    /// The caller presented no verified token, so there is no identity to stamp.
    #[error("this target needs a verified token to stamp the caller's identity")]
    NoToken,
    /// The token lacks a claim a template needs, or it is not a string.
    #[error("the token has no string claim `{0}`")]
    MissingClaim(String),
    /// A claim's value cannot be sent as this header.
    #[error("the token's value for header `{0}` is not a valid header value")]
    BadHeaderValue(String),
    /// The arguments are present but are not a JSON object.
    #[error("tool arguments must be an object to carry the caller's identity")]
    NotAnObject,
}

/// A value made of literal text and claim references.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Template(Vec<Part>);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Claim(String),
}

impl Template {
    fn parse(value: &str, at: &str) -> Result<Self, IdentityError> {
        let fail = |value: &str, problem| IdentityError {
            at: at.to_string(),
            value: value.to_string(),
            problem,
        };
        let mut parts = Vec::new();
        let mut rest = value;
        while let Some(open) = rest.find("{{") {
            if open > 0 {
                parts.push(Part::Text(rest[..open].to_string()));
            }
            let after = &rest[open + 2..];
            let close = after
                .find("}}")
                .ok_or_else(|| fail(value, "an unclosed `{{`"))?;
            let reference = after[..close].trim();
            let claim = reference
                .strip_prefix(NAMESPACE)
                .ok_or_else(|| fail(reference, "only `jwt.<claim>` resolves here"))?;
            if claim.is_empty() {
                return Err(fail(reference, "a claim name is missing after `jwt.`"));
            }
            parts.push(Part::Claim(claim.to_string()));
            rest = &after[close + 2..];
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_string()));
        }
        Ok(Template(parts))
    }

    fn needs_claims(&self) -> bool {
        self.0.iter().any(|p| matches!(p, Part::Claim(_)))
    }

    fn resolve(&self, claims: Option<&serde_json::Value>) -> Result<String, StampError> {
        let mut out = String::new();
        for part in &self.0 {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Claim(name) => {
                    let claims = claims.ok_or(StampError::NoToken)?;
                    let value = claims
                        .get(name)
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| StampError::MissingClaim(name.clone()))?;
                    out.push_str(value);
                }
            }
        }
        Ok(out)
    }
}

/// A compiled [`McpIdentity`]. Empty when the target configures none.
#[derive(Debug, Clone, Default)]
pub struct Identity {
    strip: Vec<String>,
    set: BTreeMap<String, Template>,
    headers: Vec<(HeaderName, Template)>,
}

impl Identity {
    /// Compile a target's identity block. `http` says whether the target has
    /// headers at all: `headers` on a `stdio` target is refused rather than
    /// silently going nowhere.
    ///
    /// # Errors
    /// A malformed template, a bad header name, or `headers` on a non-HTTP target.
    pub fn new(config: Option<&McpIdentity>, http: bool, at: &str) -> Result<Self, IdentityError> {
        let Some(config) = config else {
            return Ok(Self::default());
        };
        if !http && !config.headers.is_empty() {
            return Err(IdentityError {
                at: at.to_string(),
                value: "headers".to_string(),
                problem: "a stdio target has no headers to set",
            });
        }
        let set = config
            .set
            .iter()
            .map(|(name, value)| Ok((name.clone(), Template::parse(value, at)?)))
            .collect::<Result<_, IdentityError>>()?;
        let headers = config
            .headers
            .iter()
            .map(|(name, value)| {
                let header = HeaderName::try_from(name.as_str()).map_err(|_| IdentityError {
                    at: at.to_string(),
                    value: name.clone(),
                    problem: "not a valid header name",
                })?;
                Ok((header, Template::parse(value, at)?))
            })
            .collect::<Result<_, IdentityError>>()?;
        Ok(Self {
            strip: config.strip.clone(),
            set,
            headers,
        })
    }

    /// Whether this target stamps nothing.
    pub fn is_empty(&self) -> bool {
        self.strip.is_empty() && self.set.is_empty() && self.headers.is_empty()
    }

    /// Stamp one call: strip, then set arguments, then set headers, all from
    /// `claims` (the verified token). Nothing is changed unless every value
    /// resolves.
    ///
    /// # Errors
    /// [`StampError`] when the identity cannot be stamped; the call must then
    /// be refused, not sent.
    pub fn stamp(
        &self,
        arguments: &mut Option<Value>,
        headers: &mut HeaderOverride,
        claims: Option<&serde_json::Value>,
    ) -> Result<(), StampError> {
        if self.is_empty() {
            return Ok(());
        }
        let needs_claims = self.set.values().any(Template::needs_claims)
            || self.headers.iter().any(|(_, t)| t.needs_claims());
        if needs_claims && claims.is_none() {
            return Err(StampError::NoToken);
        }
        let set = self
            .set
            .iter()
            .map(|(name, t)| Ok((name, t.resolve(claims)?)))
            .collect::<Result<Vec<_>, StampError>>()?;
        let stamped_headers = self
            .headers
            .iter()
            .map(|(name, t)| {
                let value = t.resolve(claims)?;
                let value = HeaderValue::try_from(value.as_str())
                    .map_err(|_| StampError::BadHeaderValue(name.to_string()))?;
                Ok((name.clone(), value))
            })
            .collect::<Result<Vec<_>, StampError>>()?;

        let args = arguments.get_or_insert_with(Value::object);
        let map = args.as_object_mut().ok_or(StampError::NotAnObject)?;
        for name in &self.strip {
            map.remove(name.as_str());
        }
        for (name, value) in set {
            map.insert(name.clone(), Value::from(value));
        }
        for (name, value) in stamped_headers {
            headers.remove.retain(|h| h != name);
            headers.set.retain(|(h, _)| h != name);
            headers.set.push((name, value));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn identity(yaml_json: &str, http: bool) -> Result<Identity, IdentityError> {
        let config: McpIdentity = serde_json::from_str(yaml_json).unwrap();
        Identity::new(Some(&config), http, "test")
    }

    fn board() -> Identity {
        identity(
            r#"{"strip":["actor","role","agent_id","agent_key"],
                "set":{"actor":"lel-gateway/{{jwt.lel.lens}}-{{jwt.lel.claim}}"},
                "headers":{"x-lel-principal":"{{jwt.sub}}"}}"#,
            true,
        )
        .unwrap()
    }

    fn token() -> serde_json::Value {
        serde_json::json!({"iss":"orch","sub":"p/c9/adversary/1","lel.lens":"adversary","lel.claim":"c9"})
    }

    fn args(json: &str) -> Option<Value> {
        Some(Value::from_json_str(json).unwrap())
    }

    #[test]
    fn a_forged_actor_is_replaced_by_the_tokens() {
        let mut call =
            args(r#"{"id":"42","text":"hi","actor":"nano","role":"admin","agent_key":"x"}"#);
        let mut headers = HeaderOverride::default();
        board()
            .stamp(&mut call, &mut headers, Some(&token()))
            .unwrap();
        let call = call.unwrap();
        assert_eq!(
            call.get("actor").and_then(Value::as_str),
            Some("lel-gateway/adversary-c9")
        );
        assert!(call.get("role").is_none() && call.get("agent_key").is_none());
        assert_eq!(
            call.get("text").and_then(Value::as_str),
            Some("hi"),
            "other args kept"
        );
        assert_eq!(headers.set.len(), 1);
        assert_eq!(headers.set[0].0, "x-lel-principal");
        assert_eq!(headers.set[0].1, "p/c9/adversary/1");
    }

    #[test]
    fn a_call_with_no_arguments_still_gets_its_identity() {
        let mut call = None;
        board()
            .stamp(&mut call, &mut HeaderOverride::default(), Some(&token()))
            .unwrap();
        assert_eq!(
            call.unwrap().get("actor").and_then(Value::as_str),
            Some("lel-gateway/adversary-c9")
        );
    }

    #[test]
    fn no_token_or_a_missing_claim_refuses_the_call_and_changes_nothing() {
        let original = args(r#"{"actor":"nano"}"#);
        let mut call = original.clone();
        let mut headers = HeaderOverride::default();
        assert_eq!(
            board().stamp(&mut call, &mut headers, None),
            Err(StampError::NoToken)
        );
        let partial = serde_json::json!({"sub":"s","lel.lens":"adversary"});
        assert_eq!(
            board().stamp(&mut call, &mut headers, Some(&partial)),
            Err(StampError::MissingClaim("lel.claim".to_string()))
        );
        assert_eq!(call, original, "left untouched when refused");
        assert!(headers.is_empty());
    }

    #[test]
    fn a_non_string_claim_does_not_resolve() {
        let token = serde_json::json!({"sub": 7, "lel.lens":"a", "lel.claim":"c"});
        let err = board().stamp(&mut None, &mut HeaderOverride::default(), Some(&token));
        assert_eq!(err, Err(StampError::MissingClaim("sub".to_string())));
    }

    #[test]
    fn strip_alone_needs_no_token() {
        let strip = identity(r#"{"strip":["actor"]}"#, false).unwrap();
        let mut call = args(r#"{"actor":"nano","q":"x"}"#);
        strip
            .stamp(&mut call, &mut HeaderOverride::default(), None)
            .unwrap();
        assert!(call.unwrap().get("actor").is_none());
    }

    #[test]
    fn an_empty_identity_changes_nothing() {
        let none = Identity::new(None, true, "test").unwrap();
        assert!(none.is_empty());
        let original = args(r#"{"actor":"nano"}"#);
        let mut call = original.clone();
        none.stamp(&mut call, &mut HeaderOverride::default(), None)
            .unwrap();
        assert_eq!(call, original);
    }

    #[test]
    fn non_object_arguments_are_refused() {
        let mut call = args("[1,2]");
        let err = board().stamp(&mut call, &mut HeaderOverride::default(), Some(&token()));
        assert_eq!(err, Err(StampError::NotAnObject));
    }

    #[test]
    fn bad_configuration_is_refused_at_startup() {
        assert!(
            identity(r#"{"headers":{"x-a":"{{jwt.sub}}"}}"#, false).is_err(),
            "stdio headers"
        );
        assert!(
            identity(r#"{"set":{"actor":"{{sub}}"}}"#, true).is_err(),
            "no jwt. prefix"
        );
        assert!(
            identity(r#"{"set":{"actor":"{{jwt.sub"}}"#, true).is_err(),
            "unclosed"
        );
        assert!(
            identity(r#"{"set":{"actor":"{{jwt.}}"}}"#, true).is_err(),
            "empty claim"
        );
        assert!(
            identity(r#"{"headers":{"bad header":"x"}}"#, true).is_err(),
            "header name"
        );
    }

    #[test]
    fn a_header_the_route_set_or_removed_is_replaced() {
        let mut headers = HeaderOverride::default();
        headers
            .remove
            .push(HeaderName::from_static("x-lel-principal"));
        headers.set.push((
            HeaderName::from_static("x-lel-principal"),
            HeaderValue::from_static("forged"),
        ));
        board()
            .stamp(&mut None, &mut headers, Some(&token()))
            .unwrap();
        assert!(headers.remove.is_empty());
        assert_eq!(
            headers.set,
            vec![(
                HeaderName::from_static("x-lel-principal"),
                HeaderValue::from_static("p/c9/adversary/1")
            )]
        );
    }
}
