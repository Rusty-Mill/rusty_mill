//! A [`TokenValidator`] that verifies JWTs against a JWKS endpoint.
//!
//! Enabled by the `jwt` feature. This is the validator most deployments want:
//! point it at your authorization server's issuer and JWKS URI, and it verifies
//! signature, expiry, not-before, and issuer, then hands the claims to the
//! layer.
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use rusty_mcp::auth::{AuthConfig, JwtValidator};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let validator = JwtValidator::builder(
//!     "https://auth.example.com",
//!     "https://auth.example.com/.well-known/jwks.json",
//! )
//! .build()?;
//!
//! let auth = AuthConfig::new("https://mcp.example.com/mcp", Arc::new(validator))?
//!     .with_authorization_servers(["https://auth.example.com"])
//!     .with_required_scopes(["mcp:read"]);
//! # let _ = auth;
//! # Ok(())
//! # }
//! ```
//!
//! # Audience is deliberately not checked here
//!
//! This validator reads `aud` into [`VerifiedToken::audiences`] and lets
//! [`crate::auth::RequireAuthLayer`] compare it against
//! [`AuthConfig::resource`](crate::auth::AuthConfig::resource). Checking it in
//! both places would mean two sources of truth for the same value, and the one
//! that silently stopped matching would be the one nobody noticed.

use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};

use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    errors::ErrorKind,
    jwk::{Jwk, JwkSet},
};
use serde_json::Value;
use tokio::sync::RwLock;

use super::token::{TokenError, TokenValidator, ValidateFuture, VerifiedToken};

/// How long a fetched JWKS is trusted before a refresh.
const DEFAULT_JWKS_TTL: Duration = Duration::from_secs(300);

/// Floor between JWKS fetches triggered by an unrecognized `kid`.
///
/// Without this, anyone could force unbounded outbound requests to the
/// authorization server by presenting tokens with random `kid` values — a
/// cheap amplification attack through a server that is trying to be helpful.
const DEFAULT_MIN_REFETCH_INTERVAL: Duration = Duration::from_secs(30);

/// Building a [`JwtValidator`] failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum JwtValidatorError {
    /// No signing algorithms were permitted.
    #[error("at least one signing algorithm must be allowed")]
    NoAlgorithms,

    /// The HTTP client could not be constructed.
    #[error("failed to build the HTTP client: {0}")]
    Http(#[source] reqwest::Error),
}

/// Builder for [`JwtValidator`].
#[derive(Debug, Clone)]
pub struct JwtValidatorBuilder {
    issuer: String,
    jwks_uri: String,
    algorithms: Vec<Algorithm>,
    leeway_secs: u64,
    jwks_ttl: Duration,
    min_refetch_interval: Duration,
    scope_claim: String,
    request_timeout: Duration,
}

impl JwtValidatorBuilder {
    /// Permit these signing algorithms.
    ///
    /// Defaults to RS256 and ES256. Keep this list tight: accepting every
    /// algorithm an authorization server *might* use widens the attack surface
    /// for no benefit.
    pub fn with_algorithms(mut self, algorithms: impl IntoIterator<Item = Algorithm>) -> Self {
        self.algorithms = algorithms.into_iter().collect();
        self
    }

    /// Clock-skew allowance for `exp` and `nbf`, in seconds. Defaults to 60.
    pub fn with_leeway_secs(mut self, leeway_secs: u64) -> Self {
        self.leeway_secs = leeway_secs;
        self
    }

    /// How long a fetched JWKS is trusted. Defaults to five minutes.
    pub fn with_jwks_ttl(mut self, ttl: Duration) -> Self {
        self.jwks_ttl = ttl;
        self
    }

    /// Floor between refetches provoked by an unknown `kid`. Defaults to 30s.
    pub fn with_min_refetch_interval(mut self, interval: Duration) -> Self {
        self.min_refetch_interval = interval;
        self
    }

    /// Claim holding space-delimited scopes. Defaults to `scope`.
    ///
    /// Some authorization servers use `scp`, sometimes as an array; both shapes
    /// are handled.
    pub fn with_scope_claim(mut self, claim: impl Into<String>) -> Self {
        self.scope_claim = claim.into();
        self
    }

    /// Timeout for JWKS fetches. Defaults to 10 seconds.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Build the validator.
    pub fn build(self) -> Result<JwtValidator, JwtValidatorError> {
        if self.algorithms.is_empty() {
            return Err(JwtValidatorError::NoAlgorithms);
        }

        let http = reqwest::Client::builder()
            .timeout(self.request_timeout)
            .build()
            .map_err(JwtValidatorError::Http)?;

        Ok(JwtValidator {
            issuer: self.issuer,
            jwks_uri: self.jwks_uri,
            algorithms: self.algorithms,
            leeway_secs: self.leeway_secs,
            jwks_ttl: self.jwks_ttl,
            min_refetch_interval: self.min_refetch_interval,
            scope_claim: self.scope_claim,
            http,
            cache: Arc::new(RwLock::new(JwksCache::default())),
        })
    }
}

#[derive(Debug, Default)]
struct JwksCache {
    keys: Option<JwkSet>,
    fetched_at: Option<Instant>,
    last_attempt: Option<Instant>,
}

impl JwksCache {
    fn is_fresh(&self, ttl: Duration) -> bool {
        self.fetched_at.is_some_and(|at| at.elapsed() < ttl)
    }

    fn may_refetch(&self, min_interval: Duration) -> bool {
        self.last_attempt
            .is_none_or(|at| at.elapsed() >= min_interval)
    }
}

/// Verifies JWTs against a JWKS endpoint.
///
/// Clone-cheap and safe to share: the key cache sits behind an `Arc`, so all
/// clones fetch once between them.
#[derive(Debug, Clone)]
pub struct JwtValidator {
    issuer: String,
    jwks_uri: String,
    algorithms: Vec<Algorithm>,
    leeway_secs: u64,
    jwks_ttl: Duration,
    min_refetch_interval: Duration,
    scope_claim: String,
    http: reqwest::Client,
    cache: Arc<RwLock<JwksCache>>,
}

impl JwtValidator {
    /// Start building a validator for `issuer`, fetching keys from `jwks_uri`.
    ///
    /// `issuer` must match the token's `iss` claim exactly.
    pub fn builder(issuer: impl Into<String>, jwks_uri: impl Into<String>) -> JwtValidatorBuilder {
        JwtValidatorBuilder {
            issuer: issuer.into(),
            jwks_uri: jwks_uri.into(),
            algorithms: vec![Algorithm::RS256, Algorithm::ES256],
            leeway_secs: 60,
            jwks_ttl: DEFAULT_JWKS_TTL,
            min_refetch_interval: DEFAULT_MIN_REFETCH_INTERVAL,
            scope_claim: "scope".to_string(),
            request_timeout: Duration::from_secs(10),
        }
    }

    /// Fetch the JWKS and replace the cache.
    async fn fetch_jwks(&self) -> Result<JwkSet, TokenError> {
        {
            let mut cache = self.cache.write().await;
            cache.last_attempt = Some(Instant::now());
        }

        let response = self
            .http
            .get(&self.jwks_uri)
            .send()
            .await
            .map_err(|err| TokenError::Unavailable(format!("JWKS fetch failed: {err}")))?;

        if !response.status().is_success() {
            return Err(TokenError::Unavailable(format!(
                "JWKS endpoint returned {}",
                response.status()
            )));
        }

        let keys: JwkSet = response
            .json()
            .await
            .map_err(|err| TokenError::Unavailable(format!("malformed JWKS: {err}")))?;

        let mut cache = self.cache.write().await;
        cache.keys = Some(keys.clone());
        cache.fetched_at = Some(Instant::now());
        Ok(keys)
    }

    /// Find the key for `kid`, refreshing the cache when it looks stale.
    async fn key_for(&self, kid: &str) -> Result<Jwk, TokenError> {
        {
            let cache = self.cache.read().await;
            if cache.is_fresh(self.jwks_ttl)
                && let Some(keys) = &cache.keys
                && let Some(jwk) = keys.find(kid)
            {
                return Ok(jwk.clone());
            }
        }

        // Either the cache is cold/stale, or it is fresh but lacks this `kid` —
        // which is what a key rotation looks like from here. Refetch, but only
        // if the rate limit allows, so unknown kids cannot be used to hammer
        // the authorization server.
        let may_refetch = {
            let cache = self.cache.read().await;
            cache.may_refetch(self.min_refetch_interval) || !cache.is_fresh(self.jwks_ttl)
        };

        if may_refetch {
            let keys = self.fetch_jwks().await?;
            if let Some(jwk) = keys.find(kid) {
                return Ok(jwk.clone());
            }
        } else {
            // Serve the cached set rather than nothing: the key may be there
            // even though the freshness window lapsed.
            let cache = self.cache.read().await;
            if let Some(jwk) = cache.keys.as_ref().and_then(|keys| keys.find(kid)) {
                return Ok(jwk.clone());
            }
        }

        Err(TokenError::Invalid(format!(
            "no signing key matches kid `{kid}`"
        )))
    }

    /// Verify `token` and extract the claims the layer needs.
    async fn verify(&self, token: &str) -> Result<VerifiedToken, TokenError> {
        let header = decode_header(token)
            .map_err(|err| TokenError::Invalid(format!("malformed token header: {err}")))?;

        // Pin the algorithm to the allow-list before touching any key. This is
        // what prevents an attacker choosing the algorithm — the `alg: none`
        // and RS256-verified-as-HS256 family of attacks.
        if !self.algorithms.contains(&header.alg) {
            return Err(TokenError::Invalid(format!(
                "signing algorithm {:?} is not accepted",
                header.alg
            )));
        }

        let kid = header
            .kid
            .ok_or_else(|| TokenError::Invalid("token header has no `kid`".to_string()))?;

        let jwk = self.key_for(&kid).await?;
        let key = DecodingKey::from_jwk(&jwk)
            .map_err(|err| TokenError::Invalid(format!("unusable signing key: {err}")))?;

        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.leeway = self.leeway_secs;
        validation.validate_exp = true;
        validation.validate_nbf = true;
        // Audience is enforced by the layer against the configured resource;
        // see the module docs.
        validation.validate_aud = false;
        validation.required_spec_claims = ["exp", "iss"].iter().map(|s| s.to_string()).collect();

        let data = decode::<Value>(token, &key, &validation).map_err(|err| match err.kind() {
            ErrorKind::ExpiredSignature => TokenError::Expired,
            ErrorKind::InvalidIssuer => {
                TokenError::Invalid("the token was issued by an unexpected issuer".to_string())
            }
            ErrorKind::InvalidSignature => {
                TokenError::Invalid("the token signature is invalid".to_string())
            }
            other => TokenError::Invalid(format!("token validation failed: {other:?}")),
        })?;

        Ok(claims_to_token(data.claims, &self.scope_claim))
    }
}

impl TokenValidator for JwtValidator {
    fn validate<'a>(&'a self, token: &'a str) -> ValidateFuture<'a> {
        Box::pin(self.verify(token))
    }
}

/// Map a decoded claim set onto a [`VerifiedToken`].
fn claims_to_token(claims: Value, scope_claim: &str) -> VerifiedToken {
    let audiences = extract_audiences(claims.get("aud"));
    let scopes = extract_scopes(claims.get(scope_claim));

    let mut token = VerifiedToken::new(audiences);
    token.scopes = scopes;
    token.subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .map(str::to_string);
    token.client_id = claims
        .get("client_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    token.claims = claims;
    token
}

/// `aud` is a string or an array of strings (RFC 7519 §4.1.3).
fn extract_audiences(aud: Option<&Value>) -> Vec<String> {
    match aud {
        Some(Value::String(one)) => vec![one.clone()],
        Some(Value::Array(many)) => many
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Scopes arrive space-delimited (`scope`) or as an array (`scp`).
fn extract_scopes(scope: Option<&Value>) -> BTreeSet<String> {
    match scope {
        Some(Value::String(joined)) => joined.split_whitespace().map(str::to_string).collect(),
        Some(Value::Array(many)) => many
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => BTreeSet::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_string_audience() {
        assert_eq!(
            extract_audiences(Some(&json!("https://mcp.example.com/mcp"))),
            vec!["https://mcp.example.com/mcp".to_string()]
        );
    }

    #[test]
    fn reads_an_array_audience() {
        assert_eq!(
            extract_audiences(Some(&json!(["a", "b"]))),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn a_missing_audience_yields_none() {
        // The layer then rejects the token, which is the safe direction.
        assert!(extract_audiences(None).is_empty());
        assert!(extract_audiences(Some(&json!(42))).is_empty());
    }

    #[test]
    fn reads_both_scope_shapes() {
        let expected: BTreeSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();

        assert_eq!(extract_scopes(Some(&json!("a b"))), expected);
        assert_eq!(extract_scopes(Some(&json!(["a", "b"]))), expected);
        assert!(extract_scopes(None).is_empty());
    }

    #[test]
    fn maps_claims_onto_the_token() {
        let token = claims_to_token(
            json!({
                "sub": "user-1",
                "aud": "https://mcp.example.com/mcp",
                "scope": "mcp:read mcp:write",
                "client_id": "cli-9",
                "custom": true
            }),
            "scope",
        );

        assert_eq!(token.subject.as_deref(), Some("user-1"));
        assert_eq!(token.client_id.as_deref(), Some("cli-9"));
        assert_eq!(token.audiences, vec!["https://mcp.example.com/mcp"]);
        assert!(token.scopes.contains("mcp:write"));
        // The validator never claims to have checked the audience itself.
        assert!(!token.audience_verified);
        // Raw claims stay available for tools that need more.
        assert_eq!(token.claims["custom"], json!(true));
    }

    #[test]
    fn requires_at_least_one_algorithm() {
        let result = JwtValidator::builder("https://auth.example.com", "https://auth/jwks")
            .with_algorithms([])
            .build();

        assert!(matches!(result, Err(JwtValidatorError::NoAlgorithms)));
    }

    #[test]
    fn defaults_to_asymmetric_algorithms_only() {
        // Symmetric algorithms have no place with a JWKS: the shared secret
        // would have to be published as a key.
        let builder = JwtValidator::builder("https://auth.example.com", "https://auth/jwks");
        assert_eq!(builder.algorithms, vec![Algorithm::RS256, Algorithm::ES256]);
    }

    #[tokio::test]
    async fn rate_limits_refetches_provoked_by_unknown_kids() {
        let cache = JwksCache {
            keys: Some(JwkSet { keys: vec![] }),
            fetched_at: Some(Instant::now()),
            last_attempt: Some(Instant::now()),
        };

        // Fresh cache, attempt just made: a bogus `kid` must not trigger
        // another outbound fetch.
        assert!(cache.is_fresh(DEFAULT_JWKS_TTL));
        assert!(!cache.may_refetch(DEFAULT_MIN_REFETCH_INTERVAL));
    }

    #[tokio::test]
    async fn a_cold_cache_always_permits_a_fetch() {
        let cache = JwksCache::default();
        assert!(!cache.is_fresh(DEFAULT_JWKS_TTL));
        assert!(cache.may_refetch(DEFAULT_MIN_REFETCH_INTERVAL));
    }
}
