//! Access token validation.
//!
//! The scaffold does not decide *how* you validate a token — JWT signature
//! checks, RFC 7662 introspection, or a lookup in your own store are all
//! reasonable. Implement [`TokenValidator`] and the layer handles the rest of
//! the resource-server contract around it.

use std::{
    collections::{BTreeSet, HashMap},
    future::Future,
    pin::Pin,
};

/// Future returned by [`TokenValidator::validate`].
///
/// Boxed so the trait stays object-safe and validators can be stored behind an
/// `Arc<dyn TokenValidator>`.
pub type ValidateFuture<'a> =
    Pin<Box<dyn Future<Output = Result<VerifiedToken, TokenError>> + Send + 'a>>;

/// Validates bearer tokens presented to the MCP endpoint.
///
/// Implementations must reject tokens that fail signature, expiry, or issuer
/// checks. Audience binding is enforced separately by the layer — see
/// [`VerifiedToken::audiences`].
pub trait TokenValidator: Send + Sync + 'static {
    /// Validate `token`, returning its claims.
    fn validate<'a>(&'a self, token: &'a str) -> ValidateFuture<'a>;
}

/// Why a token was rejected.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TokenError {
    /// Malformed, badly signed, or otherwise unusable. Answered with `401`.
    #[error("{0}")]
    Invalid(String),

    /// Well-formed but past its expiry. Answered with `401`.
    #[error("the access token has expired")]
    Expired,

    /// The validator itself failed — the introspection endpoint was down, a
    /// JWKS fetch timed out. Answered with `503`, never `401`: the client's
    /// token may be perfectly good, and telling it to re-authorize would send
    /// the user through a pointless login.
    #[error("token validation is unavailable: {0}")]
    Unavailable(String),
}

/// A token that passed validation, and the claims the layer acts on.
///
/// Placed in the request's HTTP extensions on success, so tools can read it —
/// see the crate-level docs on per-tool scope checks.
#[derive(Debug, Clone, Default)]
pub struct VerifiedToken {
    /// `sub` — who the token was issued for.
    pub subject: Option<String>,
    /// `client_id` — which OAuth client presented it.
    pub client_id: Option<String>,
    /// Granted scopes, already split on whitespace.
    pub scopes: BTreeSet<String>,
    /// `aud` — who the token was minted for.
    ///
    /// The layer checks this against the configured canonical resource URI and
    /// rejects a mismatch. That check is what stops this server accepting a
    /// token minted for somebody else — the confused-deputy failure the spec
    /// calls out with "MCP servers **MUST NOT** accept or transit any other
    /// tokens".
    pub audiences: Vec<String>,
    /// Set when the validator already bound the token to this resource server
    /// and the layer should not re-check `audiences`.
    ///
    /// Only set this if the check genuinely happened. See
    /// [`VerifiedToken::audience_checked_by_validator`].
    pub audience_verified: bool,
    /// Everything else the validator decoded, for tools that need more.
    pub claims: serde_json::Value,
}

impl VerifiedToken {
    /// A token carrying `audiences`, to be checked by the layer.
    pub fn new(audiences: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            audiences: audiences.into_iter().map(Into::into).collect(),
            claims: serde_json::Value::Null,
            ..Default::default()
        }
    }

    /// A token whose audience the validator has already bound to this server.
    ///
    /// Use this only when the validator provably rejects tokens minted for
    /// other resources — for example an introspection endpoint scoped to this
    /// resource. Reaching for it to silence a rejection re-opens the
    /// confused-deputy hole the check exists to close.
    pub fn audience_checked_by_validator() -> Self {
        Self {
            audience_verified: true,
            claims: serde_json::Value::Null,
            ..Default::default()
        }
    }

    /// Set the subject.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Set the client id.
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    /// Add scopes from an iterator.
    pub fn with_scopes(mut self, scopes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.scopes.extend(scopes.into_iter().map(Into::into));
        self
    }

    /// Add scopes from a space-delimited `scope` claim.
    pub fn with_scope_claim(self, scope: &str) -> Self {
        self.with_scopes(scope.split_whitespace())
    }

    /// Attach the raw claim set.
    pub fn with_claims(mut self, claims: serde_json::Value) -> Self {
        self.claims = claims;
        self
    }

    /// Whether every scope in `required` is present.
    ///
    /// Plain set containment. The spec asks servers to account for scope
    /// hierarchies where a broader scope implies narrower ones; if yours has
    /// one, expand the implied scopes in your validator so they land in
    /// [`VerifiedToken::scopes`].
    pub fn has_scopes(&self, required: &BTreeSet<String>) -> bool {
        required.is_subset(&self.scopes)
    }

    /// Scopes in `required` that this token lacks.
    pub fn missing_scopes(&self, required: &BTreeSet<String>) -> Vec<String> {
        required.difference(&self.scopes).cloned().collect()
    }
}

/// An in-memory validator mapping opaque token strings to claims.
///
/// For tests and local development. It performs no cryptography and no expiry
/// checks, so it has no business in production.
#[derive(Debug, Default)]
pub struct StaticTokenValidator {
    tokens: HashMap<String, VerifiedToken>,
}

impl StaticTokenValidator {
    /// An empty validator that rejects everything.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `token` as resolving to `claims`.
    pub fn with_token(mut self, token: impl Into<String>, claims: VerifiedToken) -> Self {
        self.tokens.insert(token.into(), claims);
        self
    }
}

impl TokenValidator for StaticTokenValidator {
    fn validate<'a>(&'a self, token: &'a str) -> ValidateFuture<'a> {
        let result = self
            .tokens
            .get(token)
            .cloned()
            .ok_or_else(|| TokenError::Invalid("unrecognized access token".to_string()));
        Box::pin(std::future::ready(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn splits_the_scope_claim_on_whitespace() {
        let token = VerifiedToken::new(["https://mcp.example.com/mcp"])
            .with_scope_claim("files:read  files:write\tmail:send");
        assert_eq!(
            token.scopes,
            scopes(&["files:read", "files:write", "mail:send"])
        );
    }

    #[test]
    fn reports_missing_scopes() {
        let token = VerifiedToken::new(["r"]).with_scopes(["files:read"]);

        assert!(token.has_scopes(&scopes(&["files:read"])));
        assert!(token.has_scopes(&scopes(&[])));
        assert!(!token.has_scopes(&scopes(&["files:read", "files:write"])));
        assert_eq!(
            token.missing_scopes(&scopes(&["files:read", "files:write"])),
            vec!["files:write".to_string()]
        );
    }

    #[tokio::test]
    async fn static_validator_rejects_unknown_tokens() {
        let validator = StaticTokenValidator::new().with_token("good", VerifiedToken::new(["r"]));

        assert!(validator.validate("good").await.is_ok());
        assert!(matches!(
            validator.validate("bad").await,
            Err(TokenError::Invalid(_))
        ));
    }
}
