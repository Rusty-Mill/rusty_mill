//! A [`TokenValidator`] that verifies JWTs against a JWKS endpoint.
//!
//! Enabled by the `jwt` feature. This is the validator most deployments want:
//! point it at your authorization server's issuer and JWKS URI, and it verifies
//! signature, expiry, not-before, and issuer, then hands the claims to the
//! audience and scope check.
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
//! This validator reads `aud` into [`VerifiedToken::audiences`] and leaves its
//! caller to compare it against
//! [`AuthConfig::resource`](crate::auth::AuthConfig::resource), as
//! `serve::AuthorizedHandler` does. Checking it in
//! both places would mean two sources of truth for the same value, and the one
//! that silently stopped matching would be the one nobody noticed.
//!
//! # Algorithms and keys
//!
//! Only RS256 and ES256 exist ([`JwtAlgorithm`]); `none` and the HMAC
//! algorithms cannot be configured. A token's `alg` is checked against the
//! allow-list before any key is looked up, the key must be of the family the
//! algorithm needs, and the token must carry a `kid`. RSA moduli must be 2048
//! to 4096 bits, because the key arrives in a network document and each
//! verification costs more the larger it is.
//!
//! Time claims accept unsigned integers and finite nonnegative fractions below
//! 2^64, rounded to seconds as in the former verifier. `exp` is required; `nbf`
//! may be absent, but a present malformed time claim is rejected. Expiry exactly
//! at `now - leeway` is accepted. Arithmetic cannot overflow on extreme dates.

use std::{
    collections::BTreeSet,
    fmt,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusty_json::Value;
use rusty_oauth::{
    jwks::{Jwk, JwkSet},
    jwt::{decode_unverified, es256, rsa},
};
use rusty_tokio::sync::RwLock;

use super::token::{TokenError, TokenValidator, ValidateFuture, VerifiedToken};

/// How long a fetched JWKS is trusted before a refresh.
const DEFAULT_JWKS_TTL: Duration = Duration::from_secs(300);

/// Floor between JWKS fetches triggered by an unrecognized `kid`.
///
/// Without this, anyone could force unbounded outbound requests to the
/// authorization server by presenting tokens with random `kid` values — a
/// cheap amplification attack through a server that is trying to be helpful.
const DEFAULT_MIN_REFETCH_INTERVAL: Duration = Duration::from_secs(30);

/// The most bytes read from a JWKS endpoint. A real provider's key set can be
/// large: Bot Framework's lists 260 keys in about 910 KB, close to
/// `rusty_request`'s 1 MiB default. The document is parsed and held in
/// memory, so this leaves room to grow (about 4.6 times that) without being
/// unbounded; a larger one fails the fetch ("JWKS fetch failed").
const MAX_JWKS_BODY_LEN: u64 = 4 << 20;

/// Leeway is clamped to this many seconds, well inside the range the claim
/// arithmetic can add to a timestamp without overflowing.
const MAX_LEEWAY_SECS: u64 = i32::MAX as u64;

/// A signing algorithm a [`JwtValidator`] may be told to accept.
///
/// Only asymmetric algorithms exist, on purpose. A shared-secret algorithm
/// such as `HS256` checked against a key published in a JWKS is the root of
/// the classic algorithm-confusion forgery, and `none` is never acceptable:
/// with no variant for either, no configuration can switch them on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JwtAlgorithm {
    /// RSASSA-PKCS1-v1_5 with SHA-256 (RFC 7518 §3.3), for RSA keys of 2048
    /// to 4096 bits.
    Rs256,
    /// ECDSA over P-256 with SHA-256 (RFC 7518 §3.4).
    Es256,
}

impl JwtAlgorithm {
    /// The algorithm a token's `alg` header names, if it is one we implement.
    fn from_header_name(name: &str) -> Option<Self> {
        match name {
            "RS256" => Some(Self::Rs256),
            "ES256" => Some(Self::Es256),
            _ => None,
        }
    }
}

impl fmt::Display for JwtAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Rs256 => "RS256",
            Self::Es256 => "ES256",
        })
    }
}

/// Building a [`JwtValidator`] failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum JwtValidatorError {
    /// No signing algorithms were permitted.
    NoAlgorithms,
}

impl fmt::Display for JwtValidatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAlgorithms => f.write_str("at least one signing algorithm must be allowed"),
        }
    }
}

impl std::error::Error for JwtValidatorError {}

/// Builder for [`JwtValidator`].
#[derive(Debug, Clone)]
pub struct JwtValidatorBuilder {
    issuer: String,
    jwks_uri: String,
    algorithms: Vec<JwtAlgorithm>,
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
    pub fn with_algorithms(mut self, algorithms: impl IntoIterator<Item = JwtAlgorithm>) -> Self {
        self.algorithms = algorithms.into_iter().collect();
        self
    }

    /// Clock-skew allowance for `exp` and `nbf`, in seconds. Defaults to 60;
    /// values above `i32::MAX` are clamped to it.
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

        // The settings `reqwest` had here: a request timeout, at most 10
        // redirects, no cookie jar, no proxy from the environment. No
        // connection pool either: fetches are minutes apart, and a pooled
        // connection the server has since closed fails without a retry (and
        // would also block the next fetch for the refetch interval).
        let http = rusty_request::Client::builder()
            .timeout(self.request_timeout)
            .max_redirects(10)
            .no_cookie_store()
            .no_pool()
            .max_response_body_len(MAX_JWKS_BODY_LEN)
            .build();

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
    algorithms: Vec<JwtAlgorithm>,
    leeway_secs: u64,
    jwks_ttl: Duration,
    min_refetch_interval: Duration,
    scope_claim: String,
    http: rusty_request::Client,
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
            algorithms: vec![JwtAlgorithm::Rs256, JwtAlgorithm::Es256],
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
            .map_err(|err| TokenError::Unavailable(format!("JWKS fetch failed: {err}")))?
            .send()
            .await
            .map_err(|err| TokenError::Unavailable(format!("JWKS fetch failed: {err}")))?;

        if !response.status().is_success() {
            return Err(TokenError::Unavailable(format!(
                "JWKS endpoint returned {}",
                response.status()
            )));
        }

        let body = response
            .text()
            .map_err(|err| TokenError::Unavailable(format!("malformed JWKS: {err}")))?;
        let keys = JwkSet::parse(&body)
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

    /// The algorithm the token asks for, if this validator accepts it.
    ///
    /// Decided from the allow-list alone, before any key is looked up: that is
    /// what stops an attacker choosing the algorithm (`alg: none`, or an
    /// HMAC token "signed" with a public key).
    fn accepted_algorithm(&self, header: &Value) -> Result<JwtAlgorithm, TokenError> {
        let name = header
            .get("alg")
            .and_then(Value::as_str)
            .ok_or_else(|| TokenError::Invalid("token header has no `alg`".to_string()))?;
        JwtAlgorithm::from_header_name(name)
            .filter(|alg| self.algorithms.contains(alg))
            .ok_or_else(|| {
                // The name is the caller's, so keep it short.
                let shown: String = name.chars().take(16).collect();
                TokenError::Invalid(format!("signing algorithm `{shown}` is not accepted"))
            })
    }

    /// Verify `token` and extract the claims the audience and scope check needs.
    async fn verify(&self, token: &str) -> Result<VerifiedToken, TokenError> {
        let decoded = decode_unverified(token)
            .map_err(|err| TokenError::Invalid(format!("malformed token: {err}")))?;

        let alg = self.accepted_algorithm(&decoded.header)?;

        let kid = decoded
            .header
            .get("kid")
            .and_then(Value::as_str)
            .ok_or_else(|| TokenError::Invalid("token header has no `kid`".to_string()))?;

        let jwk = self.key_for(kid).await?;
        let claims = verify_signature(token, alg, &jwk)?;
        check_claims(&claims, &self.issuer, self.leeway_secs)?;

        Ok(claims_to_token(claims, &self.scope_claim))
    }
}

impl TokenValidator for JwtValidator {
    fn validate<'a>(&'a self, token: &'a str) -> ValidateFuture<'a> {
        Box::pin(self.verify(token))
    }
}

/// Verify the signature with a key of the family `alg` needs, and return the
/// claims. An RSA key never verifies an ES256 token or the reverse: the key
/// is converted for the algorithm, not the other way round.
fn verify_signature(token: &str, alg: JwtAlgorithm, jwk: &Jwk) -> Result<Value, TokenError> {
    let verified = match alg {
        JwtAlgorithm::Rs256 => {
            let key = rsa_key(jwk)?;
            rsa::verify_rs256(token, &key)
        }
        JwtAlgorithm::Es256 => {
            let key = ec_key(jwk)?;
            es256::verify_es256(token, &key)
        }
    };
    verified.map_err(|_| TokenError::Invalid("the token signature is invalid".to_string()))
}

fn unusable_key(reason: impl fmt::Display) -> TokenError {
    TokenError::Invalid(format!("unusable signing key: {reason}"))
}

/// P-256 JWK coordinates are fixed-width octet strings, not variable-width
/// integers. Check their encoding before the primitive's range/on-curve checks.
fn ec_key(jwk: &Jwk) -> Result<es256::EcPublicKey, TokenError> {
    let components = jwk.ec_components().map_err(unusable_key)?;
    if components.crv != "P-256" {
        return Err(unusable_key("the EC curve is not P-256"));
    }
    let x = rusty_base64::decode_url_safe(&components.x).map_err(unusable_key)?;
    let y = rusty_base64::decode_url_safe(&components.y).map_err(unusable_key)?;
    if x.len() != 32 || y.len() != 32 {
        return Err(unusable_key("P-256 coordinates must each be 32 bytes"));
    }
    es256::EcPublicKey::from_affine_coordinates(&x, &y).map_err(unusable_key)
}

/// An RSA verification key from a JWK, refused if `rusty_oauth` finds the
/// modulus or exponent unusable, before any arithmetic is done with them.
fn rsa_key(jwk: &Jwk) -> Result<rsa::RsaPublicKey, TokenError> {
    if jwk.kty != "RSA" {
        return Err(unusable_key("the key is not an RSA key"));
    }
    let component = |name: &str| -> Result<Vec<u8>, TokenError> {
        let encoded = jwk
            .raw
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| unusable_key(format!("the RSA key has no `{name}`")))?;
        rusty_base64::decode_url_safe(encoded)
            .map_err(|err| unusable_key(format!("RSA `{name}` is not base64url: {err}")))
    };
    let n = component("n")?;
    let e = component("e")?;

    // `rusty_oauth` owns the key policy (modulus 2048 to 4096 bits, no leading
    // zero byte, odd modulus, odd exponent from 3 to 33 bits). Asking now keeps
    // the early, specific "unusable signing key" answer instead of a generic
    // "signature is invalid" at verification.
    let key = rsa::RsaPublicKey::from_components(&n, &e);
    match key.unusable_reason() {
        Some(reason) => Err(unusable_key(reason)),
        None => Ok(key),
    }
}

/// Parse the former jsonwebtoken 11 NumericDate contract: unsigned integers
/// remain exact; finite nonnegative floats below 2^64 round to whole seconds.
/// Absence is handled by the caller; a present invalid value never disappears.
fn numeric_date(value: &Value) -> Result<u64, TokenError> {
    if let Some(seconds) = value.as_u64() {
        return Ok(seconds);
    }
    if let Some(seconds) = value.as_f64()
        && seconds.is_finite()
        && seconds >= 0.0
        && seconds < u64::MAX as f64
    {
        return Ok(seconds.round() as u64);
    }
    Err(TokenError::Invalid(
        "token validation failed: invalid NumericDate".to_string(),
    ))
}

/// Check `exp`, `nbf` and `iss`. Audience is left to the caller's check.
fn check_claims(claims: &Value, issuer: &str, leeway_secs: u64) -> Result<(), TokenError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    check_claims_at(claims, issuer, leeway_secs, now)
}

fn check_claims_at(
    claims: &Value,
    issuer: &str,
    leeway_secs: u64,
    now: u64,
) -> Result<(), TokenError> {
    let exp = claims.get("exp").ok_or_else(|| {
        TokenError::Invalid("token validation failed: missing expiry".to_string())
    })?;
    let exp = numeric_date(exp)?;
    let nbf = claims.get("nbf").map(numeric_date).transpose()?;
    // Widen before addition: even u64::MAX timestamps cannot overflow, and
    // subtracting a large leeway from a small clock cannot underflow.
    let leeway = u128::from(leeway_secs.min(MAX_LEEWAY_SECS));
    let now = u128::from(now);
    if u128::from(exp) + leeway < now {
        return Err(TokenError::Expired);
    }
    if nbf.is_some_and(|nbf| u128::from(nbf) > now + leeway) {
        return Err(TokenError::Invalid(
            "the token is not yet valid".to_string(),
        ));
    }
    if claims.get("iss").and_then(Value::as_str) != Some(issuer) {
        return Err(TokenError::Invalid(
            "the token was issued by an unexpected issuer".to_string(),
        ));
    }
    Ok(())
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
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use rusty_json::json;

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
        // The audience check then rejects the token, which is the safe direction.
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
        assert_eq!(
            builder.algorithms,
            vec![JwtAlgorithm::Rs256, JwtAlgorithm::Es256]
        );
    }

    #[rusty_tokio::test]
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

    #[rusty_tokio::test]
    async fn a_cold_cache_always_permits_a_fetch() {
        let cache = JwksCache::default();
        assert!(!cache.is_fresh(DEFAULT_JWKS_TTL));
        assert!(cache.may_refetch(DEFAULT_MIN_REFETCH_INTERVAL));
    }

    const ISS: &str = "https://auth.example.com";

    fn now_secs() -> i64 {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        i64::try_from(secs).expect("a plausible time")
    }

    fn b64(bytes: &[u8]) -> String {
        rusty_base64::encode_url_safe_no_pad(bytes)
    }

    fn jwk(json: &str) -> Jwk {
        JwkSet::parse(&format!(r#"{{"keys":[{json}]}}"#))
            .expect("a JWKS")
            .keys
            .remove(0)
    }

    fn rsa_jwk(n: &[u8], e: &[u8]) -> Jwk {
        jwk(&format!(
            r#"{{"kty":"RSA","kid":"k","n":"{}","e":"{}"}}"#,
            b64(n),
            b64(e)
        ))
    }

    /// A modulus of `bytes` bytes with its top bit set, so it is `bytes * 8` bits.
    fn modulus_of(bytes: usize) -> Vec<u8> {
        vec![0xFF; bytes]
    }

    fn reason(err: TokenError) -> String {
        match err {
            TokenError::Invalid(why) => why,
            other => panic!("expected Invalid, got {other}"),
        }
    }

    fn default_validator() -> JwtValidator {
        JwtValidator::builder(ISS, "https://auth.example.com/jwks")
            .build()
            .expect("build")
    }

    #[test]
    fn zero_prefixed_p256_coordinates_must_retain_all_32_bytes() {
        // Independent P-256 points generated with Python cryptography/OpenSSL
        // from the small test scalars noted below. Removing the leading zero
        // preserves the mathematical point, but violates the JWK encoding.
        for (coordinate, x, y) in [
            // d = 379
            (
                "x",
                "AFVDiUrz0A7X10Cr29dclrBod7eH219w7qeLkKjXwAo",
                "u0yFo9jqKe-q-iRAaRLdhNWxTcMr9lbvbGvVil2UP5I",
            ),
            // d = 43
            (
                "y",
                "mGriUG8f8QTQQjCGHY9LSY9LxMbQCbMPdUTcEpuC0o0",
                "ADzMwKZGDgrjKKTZfTx7YdhvxiicGJ8lJREMRBuwfpc",
            ),
        ] {
            let mut key = jwk(&format!(
                r#"{{"kty":"EC","crv":"P-256","x":"{x}","y":"{y}"}}"#
            ));
            assert!(ec_key(&key).is_ok());
            let bytes = rusty_base64::decode_url_safe(
                key.raw[coordinate].as_str().expect("coordinate string"),
            )
            .expect("decode coordinate");
            assert_eq!(bytes[0], 0);
            key.raw[coordinate] = json!(b64(&bytes[1..]));
            assert!(reason(ec_key(&key).expect_err("short coordinate")).contains("32 bytes"));
        }
    }

    #[test]
    fn bounded_even_moduli_are_refused_before_arithmetic() {
        let mut even = modulus_of(256);
        even[255] = 0xfe;
        let mut power_of_two = vec![0; 256];
        power_of_two[0] = 0x80;
        for n in [even, power_of_two] {
            assert!(
                reason(rsa_key(&rsa_jwk(&n, &[1, 0, 1])).expect_err("even modulus"))
                    .contains("even")
            );
        }
    }

    #[test]
    fn numeric_dates_preserve_rounding_and_boundaries_without_overflow() {
        for (value, expected) in [
            (json!(1000.49), 1000),
            (json!(1000.5), 1001),
            (json!(4102444800.0), 4102444800),
            (json!(u64::MAX), u64::MAX),
        ] {
            assert_eq!(numeric_date(&value).expect("valid NumericDate"), expected);
        }
        for value in [
            json!(-1),
            json!(-0.5),
            json!(18446744073709551616.0),
            json!(1e100),
            json!("1000"),
            json!(null),
            json!(true),
            json!([]),
            json!({}),
        ] {
            assert!(numeric_date(&value).is_err(), "{value:?}");
        }
        // Exact expiry boundary retains the previous verifier's acceptance.
        let claims = json!({"iss": ISS, "exp": 940, "nbf": 1060});
        assert!(check_claims_at(&claims, ISS, 60, 1000).is_ok());
        assert!(matches!(
            check_claims_at(&claims, ISS, 60, 1001),
            Err(TokenError::Expired)
        ));
        let claims = json!({"iss": ISS, "exp": u64::MAX, "nbf": u64::MAX});
        assert!(check_claims_at(&claims, ISS, u64::MAX, u64::MAX).is_ok());
        assert!(check_claims_at(&claims, ISS, u64::MAX, 0).is_err());
        let claims = json!({"iss": ISS, "exp": 0});
        assert!(check_claims_at(&claims, ISS, u64::MAX, 0).is_ok());
        assert!(matches!(
            check_claims_at(&claims, ISS, u64::MAX, MAX_LEEWAY_SECS + 1),
            Err(TokenError::Expired)
        ));
    }

    #[test]
    fn rsa_moduli_from_2048_to_4096_bits_are_accepted() {
        assert!(rsa_key(&rsa_jwk(&modulus_of(256), &[1, 0, 1])).is_ok());
        assert!(rsa_key(&rsa_jwk(&modulus_of(512), &[1, 0, 1])).is_ok());
    }

    #[test]
    fn rsa_moduli_outside_that_range_are_refused() {
        // 255 bytes is 2040 bits; 513 bytes is 4104.
        for bytes in [0, 1, 128, 255, 513, 1024] {
            let err = rsa_key(&rsa_jwk(&modulus_of(bytes), &[1, 0, 1])).expect_err("out of range");
            assert!(reason(err).contains("bits"), "{bytes} bytes");
        }
    }

    #[test]
    fn a_modulus_with_a_leading_zero_byte_is_refused() {
        let mut n = vec![0];
        n.extend(modulus_of(256));
        let err = rsa_key(&rsa_jwk(&n, &[1, 0, 1])).expect_err("leading zero");
        assert!(reason(err).contains("leading zero"));
    }

    #[test]
    fn only_odd_exponents_up_to_33_bits_are_accepted() {
        let n = modulus_of(256);
        for e in [vec![3], vec![1, 0, 1], vec![1, 0, 0, 0, 1]] {
            assert!(rsa_key(&rsa_jwk(&n, &e)).is_ok(), "{e:?}");
        }
        // Empty, 0, 1, even, 2^33 + 1, and one byte too long.
        for e in [
            vec![],
            vec![0],
            vec![1],
            vec![2],
            vec![4],
            vec![1, 0, 0, 0, 0],
            vec![2, 0, 0, 0, 1],
            vec![1, 0, 0, 0, 0, 1],
        ] {
            let err = rsa_key(&rsa_jwk(&n, &e)).expect_err("bad exponent");
            assert!(reason(err).contains("exponent"), "{e:?}");
        }
    }

    #[test]
    fn an_ec_key_is_not_an_rsa_key() {
        let key = jwk(r#"{"kty":"EC","crv":"P-256","kid":"k","x":"AA","y":"AA"}"#);
        let err = rsa_key(&key).expect_err("wrong family");
        assert!(reason(err).contains("not an RSA key"));
    }

    #[test]
    fn claims_that_are_in_order_pass() {
        let claims = json!({"iss": ISS, "exp": now_secs() + 600});
        assert!(check_claims(&claims, ISS, 60).is_ok());
    }

    #[test]
    fn an_expired_token_is_reported_as_expired() {
        let claims = json!({"iss": ISS, "exp": now_secs() - 3600});
        assert!(matches!(
            check_claims(&claims, ISS, 60),
            Err(TokenError::Expired)
        ));
    }

    #[test]
    fn leeway_covers_a_slightly_expired_token_and_no_more() {
        let within = json!({"iss": ISS, "exp": now_secs() - 30});
        assert!(check_claims(&within, ISS, 60).is_ok());

        let beyond = json!({"iss": ISS, "exp": now_secs() - 120});
        assert!(matches!(
            check_claims(&beyond, ISS, 60),
            Err(TokenError::Expired)
        ));
    }

    #[test]
    fn a_future_nbf_is_not_yet_valid_unless_leeway_covers_it() {
        let later = json!({"iss": ISS, "exp": now_secs() + 7200, "nbf": now_secs() + 3600});
        let err = check_claims(&later, ISS, 60).expect_err("not yet valid");
        assert!(reason(err).contains("not yet valid"));

        let soon = json!({"iss": ISS, "exp": now_secs() + 7200, "nbf": now_secs() + 30});
        assert!(check_claims(&soon, ISS, 60).is_ok());
    }

    #[test]
    fn a_wrong_or_missing_issuer_is_refused() {
        let exp = now_secs() + 600;
        let wrong = json!({"iss": "https://evil.example.com", "exp": exp});
        assert!(
            reason(check_claims(&wrong, ISS, 60).expect_err("wrong")).contains("unexpected issuer")
        );

        let missing = json!({"exp": exp});
        assert!(check_claims(&missing, ISS, 60).is_err());

        let not_a_string = json!({"iss": 7, "exp": exp});
        assert!(check_claims(&not_a_string, ISS, 60).is_err());
    }

    #[test]
    fn a_missing_or_malformed_exp_is_refused() {
        for claims in [
            json!({"iss": ISS}),
            json!({"iss": ISS, "exp": "soon"}),
            json!({"iss": ISS, "exp": null}),
        ] {
            assert!(check_claims(&claims, ISS, 60).is_err(), "{claims:?}");
        }
    }

    #[test]
    fn an_enormous_leeway_is_clamped_not_overflowed() {
        let valid = json!({"iss": ISS, "exp": now_secs() + 600});
        assert!(check_claims(&valid, ISS, u64::MAX).is_ok());

        // Extreme timestamps against the clamped leeway must not overflow either.
        let far_future_nbf = json!({"iss": ISS, "exp": i64::MAX, "nbf": i64::MAX});
        assert!(check_claims(&far_future_nbf, ISS, u64::MAX).is_err());
        let ancient_exp = json!({"iss": ISS, "exp": i64::MIN});
        assert!(matches!(
            check_claims(&ancient_exp, ISS, u64::MAX),
            Err(TokenError::Invalid(_))
        ));
    }

    #[test]
    fn only_the_configured_asymmetric_algorithms_are_accepted() {
        let validator = default_validator();
        assert_eq!(
            validator.accepted_algorithm(&json!({"alg": "RS256"})).ok(),
            Some(JwtAlgorithm::Rs256)
        );
        assert_eq!(
            validator.accepted_algorithm(&json!({"alg": "ES256"})).ok(),
            Some(JwtAlgorithm::Es256)
        );
        for alg in [
            "none", "None", "HS256", "HS384", "RS384", "PS256", "EdDSA", "rs256", "",
        ] {
            assert!(
                validator.accepted_algorithm(&json!({"alg": alg})).is_err(),
                "{alg:?}"
            );
        }
        assert!(validator.accepted_algorithm(&json!({})).is_err());
        assert!(validator.accepted_algorithm(&json!({"alg": 256})).is_err());
    }

    #[test]
    fn a_narrower_allow_list_is_respected() {
        let validator = JwtValidator::builder(ISS, "https://auth.example.com/jwks")
            .with_algorithms([JwtAlgorithm::Es256])
            .build()
            .expect("build");
        assert!(
            validator
                .accepted_algorithm(&json!({"alg": "RS256"}))
                .is_err()
        );
        assert!(
            validator
                .accepted_algorithm(&json!({"alg": "ES256"}))
                .is_ok()
        );
    }

    #[test]
    fn a_long_algorithm_name_is_cut_short_in_the_message() {
        let validator = default_validator();
        let err = validator
            .accepted_algorithm(&json!({"alg": "A".repeat(1000)}))
            .expect_err("unknown algorithm");
        assert!(reason(err).len() < 80);
    }
}
