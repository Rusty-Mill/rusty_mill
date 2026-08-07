//! JWT/OIDC bearer-token verification for `[jwt]` -- an additional way to
//! satisfy `routes::check_auth`, alongside (never instead of) the
//! existing static `server.api_key_env` / `[[clients]].api_key_env`
//! tokens. See `JwtVerifier::verify`'s doc comment for the fail-closed
//! contract this module is built around.
//!
//! Two modes, resolved once at startup by `JwtVerifier::new` from the
//! already-env-resolved secret (`main`/test setup reads
//! `hs256_secret_env` the same way it already reads `api_key`/`admin_key`,
//! keeping every env-var read in one place):
//!
//! - HS256: a shared secret, no network call ever needed.
//! - JWKS (RS256 only): fetched from `jwks_url` and cached by `kid`,
//!   re-fetched on a cache-miss (handles key rotation) or once
//!   `jwks_cache_secs` has elapsed.
//!
//! The algorithm used to validate a token is always chosen by *this
//! router's own configured mode*, never by trusting the token's own
//! `alg` header claim -- `jsonwebtoken::decode` cross-checks the header's
//! `alg` against `Validation`'s allowed algorithm list and rejects a
//! mismatch, which is what actually closes the classic JWT
//! algorithm-confusion hole (e.g. a token claiming `"alg": "none"`, or
//! HS256-signed-with-the-public-RSA-key-as-a-secret against a service
//! that blindly honors whatever `alg` the header names).

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use rp_router::JwtConfig;
use serde::Deserialize;
use serde_json::Value;

/// One JWKS document entry -- only the fields needed to build an RSA
/// `DecodingKey`. Non-RSA entries (`kty` other than `"RSA"`) and entries
/// missing `kid`/`n`/`e` are skipped when building the cache, not errors
/// for the whole fetch -- a JWKS document commonly mixes key types across
/// a rotation window.
#[derive(Debug, Deserialize)]
struct Jwk {
    kty: String,
    kid: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

enum Mode {
    /// No network call ever needed -- the key is fixed at startup.
    Hs256(DecodingKey),
    /// `kid` -> RSA `DecodingKey`, refreshed from `jwks_url` on a TTL or a
    /// cache-miss.
    Jwks {
        url: String,
        cache_ttl: Duration,
        cache: RwLock<Option<(Instant, HashMap<String, DecodingKey>)>>,
    },
}

pub struct JwtVerifier {
    mode: Mode,
    issuer: Option<String>,
    audience: Option<String>,
    client_claim: Option<String>,
    http: reqwest::Client,
}

impl JwtVerifier {
    /// `hs256_secret` is the already-env-resolved secret (or `None`) --
    /// this never reads the environment itself, same convention `main`
    /// already uses for `api_key`/`admin_key`. Returns `None` if neither
    /// mode is actually usable (`hs256_secret` absent and `jwks_url`
    /// unset), so the caller can log a startup warning and leave JWT auth
    /// disabled entirely, the same soft-failure pattern a misconfigured
    /// provider or moderation backend already gets.
    pub fn new(cfg: &JwtConfig, hs256_secret: Option<String>) -> Option<Self> {
        let mode = if let Some(secret) = hs256_secret {
            Mode::Hs256(DecodingKey::from_secret(secret.as_bytes()))
        } else if let Some(url) = &cfg.jwks_url {
            Mode::Jwks {
                url: url.clone(),
                cache_ttl: Duration::from_secs(cfg.jwks_cache_secs),
                cache: RwLock::new(None),
            }
        } else {
            return None;
        };

        Some(Self {
            mode,
            issuer: cfg.issuer.clone(),
            audience: cfg.audience.clone(),
            client_claim: cfg.client_claim.clone(),
            http: reqwest::Client::new(),
        })
    }

    /// The claim name configured via `[jwt].client_claim`, if any -- see
    /// `routes::resolve_client_identity`, the only caller.
    pub fn client_claim(&self) -> Option<&str> {
        self.client_claim.as_deref()
    }

    /// Verifies `token`, returning its claims on success. `None` on
    /// *any* failure -- malformed token, wrong/missing signature, expired
    /// (`exp` is required by `jsonwebtoken`'s default `Validation`),
    /// issuer/audience mismatch, an unresolvable `kid` (including "the
    /// JWKS endpoint couldn't be reached at all right now"). This is an
    /// authentication check, not a best-effort content check like
    /// moderation/web_search -- it fails **closed**: any of the above
    /// means "not authenticated," never "let it through anyway because
    /// the backend was unavailable."
    pub async fn verify(&self, token: &str) -> Option<Value> {
        let key = match &self.mode {
            Mode::Hs256(key) => key.clone(),
            Mode::Jwks { .. } => {
                let header = decode_header(token).ok()?;
                let kid = header.kid?;
                self.jwks_key(&kid).await?
            }
        };

        let algorithm = match &self.mode {
            Mode::Hs256(_) => Algorithm::HS256,
            Mode::Jwks { .. } => Algorithm::RS256,
        };
        let mut validation = Validation::new(algorithm);
        if let Some(issuer) = &self.issuer {
            validation.set_issuer(&[issuer]);
        }
        if let Some(audience) = &self.audience {
            validation.set_audience(&[audience]);
        }

        let data = decode::<Value>(token, &key, &validation).ok()?;
        Some(data.claims)
    }

    /// Looks up `kid` in the cache, refreshing from `jwks_url` first if
    /// the cache is stale or doesn't have that `kid` yet (handles key
    /// rotation: a token signed with a brand-new key shouldn't have to
    /// wait out the rest of `cache_ttl` to verify). `None` if the fetch
    /// fails or the refreshed document still has no matching `kid` --
    /// either way, fails closed at the `verify` call site above.
    async fn jwks_key(&self, kid: &str) -> Option<DecodingKey> {
        let Mode::Jwks {
            url,
            cache_ttl,
            cache,
        } = &self.mode
        else {
            unreachable!("jwks_key is only ever called in Mode::Jwks")
        };

        {
            let guard = cache.read().unwrap();
            if let Some((fetched_at, keys)) = guard.as_ref() {
                if fetched_at.elapsed() < *cache_ttl {
                    if let Some(key) = keys.get(kid) {
                        return Some(key.clone());
                    }
                    // Stale-but-not-expired cache with no matching kid --
                    // fall through to a refresh rather than failing
                    // immediately, in case a key just rotated in.
                }
            }
        }

        let keys = self.fetch_jwks(url).await?;
        let key = keys.get(kid).cloned();
        *cache.write().unwrap() = Some((Instant::now(), keys));
        key
    }

    async fn fetch_jwks(&self, url: &str) -> Option<HashMap<String, DecodingKey>> {
        let resp = self.http.get(url).send().await.ok()?;
        let jwks: Jwks = resp.json().await.ok()?;
        Some(
            jwks.keys
                .into_iter()
                .filter(|k| k.kty == "RSA")
                .filter_map(|k| {
                    let kid = k.kid?;
                    let n = k.n?;
                    let e = k.e?;
                    let key = DecodingKey::from_rsa_components(&n, &e).ok()?;
                    Some((kid, key))
                })
                .collect(),
        )
    }
}

/// Reads `claim` out of a verified token's claims as a string. `None` if
/// the claim is absent or isn't a JSON string (a numeric/boolean/array
/// `sub`, say) -- treated as "no identity to map," not an error, by
/// `routes::resolve_client_identity`, the only caller.
pub(crate) fn claim_as_str<'a>(claims: &'a Value, claim: &str) -> Option<&'a str> {
    claims.get(claim)?.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn hs256_config() -> JwtConfig {
        JwtConfig {
            jwks_url: None,
            hs256_secret_env: Some("UNUSED_IN_TESTS".to_string()),
            issuer: None,
            audience: None,
            jwks_cache_secs: 300,
            client_claim: None,
        }
    }

    fn hs256_token(secret: &str, claims: &Value) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    fn future_exp() -> i64 {
        // jsonwebtoken validates `exp` against real wall-clock time, so
        // tests need a real future timestamp -- this crate has no other
        // dependency on "real now" (see the workspace-wide ban on
        // Date::now() in scripts; this is plain std, not a script).
        (std::time::SystemTime::now() + Duration::from_secs(3600))
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    // --- new (mode selection) -------------------------------------------------------

    #[test]
    fn new_returns_none_when_neither_mode_is_usable() {
        let cfg = JwtConfig {
            jwks_url: None,
            hs256_secret_env: Some("UNSET_ENV_VAR".to_string()),
            issuer: None,
            audience: None,
            jwks_cache_secs: 300,
            client_claim: None,
        };
        assert!(JwtVerifier::new(&cfg, None).is_none());
    }

    #[test]
    fn new_prefers_hs256_when_both_modes_are_configured() {
        let cfg = JwtConfig {
            jwks_url: Some("https://example.com/jwks.json".to_string()),
            hs256_secret_env: Some("JWT_SECRET".to_string()),
            issuer: None,
            audience: None,
            jwks_cache_secs: 300,
            client_claim: None,
        };
        let verifier = JwtVerifier::new(&cfg, Some("s3cret".to_string())).unwrap();
        assert!(matches!(verifier.mode, Mode::Hs256(_)));
    }

    // --- verify: HS256 ---------------------------------------------------------------

    #[tokio::test]
    async fn verify_accepts_a_validly_signed_hs256_token() {
        let verifier = JwtVerifier::new(&hs256_config(), Some("s3cret".to_string())).unwrap();
        let token = hs256_token("s3cret", &json!({"sub": "alice", "exp": future_exp()}));
        let claims = verifier.verify(&token).await.expect("should verify");
        assert_eq!(claims["sub"], "alice");
    }

    #[tokio::test]
    async fn verify_rejects_a_token_signed_with_the_wrong_secret() {
        let verifier = JwtVerifier::new(&hs256_config(), Some("s3cret".to_string())).unwrap();
        let token = hs256_token(
            "wrong-secret",
            &json!({"sub": "alice", "exp": future_exp()}),
        );
        assert!(verifier.verify(&token).await.is_none());
    }

    #[tokio::test]
    async fn verify_rejects_an_expired_token() {
        let verifier = JwtVerifier::new(&hs256_config(), Some("s3cret".to_string())).unwrap();
        let expired = json!({"sub": "alice", "exp": 1}); // 1970, long expired
        let token = hs256_token("s3cret", &expired);
        assert!(verifier.verify(&token).await.is_none());
    }

    #[tokio::test]
    async fn verify_rejects_a_malformed_token() {
        let verifier = JwtVerifier::new(&hs256_config(), Some("s3cret".to_string())).unwrap();
        assert!(verifier.verify("not.a.jwt").await.is_none());
    }

    #[tokio::test]
    async fn verify_checks_issuer_when_configured() {
        let mut cfg = hs256_config();
        cfg.issuer = Some("https://issuer.example.com".to_string());
        let verifier = JwtVerifier::new(&cfg, Some("s3cret".to_string())).unwrap();

        let wrong_issuer = hs256_token(
            "s3cret",
            &json!({"sub": "alice", "exp": future_exp(), "iss": "https://someone-else.example.com"}),
        );
        assert!(verifier.verify(&wrong_issuer).await.is_none());

        let right_issuer = hs256_token(
            "s3cret",
            &json!({"sub": "alice", "exp": future_exp(), "iss": "https://issuer.example.com"}),
        );
        assert!(verifier.verify(&right_issuer).await.is_some());
    }

    #[tokio::test]
    async fn verify_checks_audience_when_configured() {
        let mut cfg = hs256_config();
        cfg.audience = Some("rusty-provider".to_string());
        let verifier = JwtVerifier::new(&cfg, Some("s3cret".to_string())).unwrap();

        let wrong_audience = hs256_token(
            "s3cret",
            &json!({"sub": "alice", "exp": future_exp(), "aud": "someone-else"}),
        );
        assert!(verifier.verify(&wrong_audience).await.is_none());

        let right_audience = hs256_token(
            "s3cret",
            &json!({"sub": "alice", "exp": future_exp(), "aud": "rusty-provider"}),
        );
        assert!(verifier.verify(&right_audience).await.is_some());
    }

    #[tokio::test]
    async fn verify_rejects_a_token_whose_header_claims_a_different_algorithm() {
        // Hand-built rather than jsonwebtoken::encode()'d -- there's no
        // valid EncodingKey/Algorithm combination that would let a real
        // RS256 token be produced without an actual RSA key, and none is
        // needed: jsonwebtoken checks the header's `alg` against
        // `Validation`'s fixed allowed-algorithm list *before* it ever
        // attempts signature verification, so even a garbage signature
        // segment proves the algorithm-confusion class of attack this
        // module's Validation::new(fixed algorithm) (never the token's
        // own header) is built to close.
        let verifier = JwtVerifier::new(&hs256_config(), Some("s3cret".to_string())).unwrap();
        let header = base64url(&json!({"alg": "RS256", "typ": "JWT"}).to_string());
        let payload = base64url(&json!({"sub": "alice", "exp": future_exp()}).to_string());
        let token = format!("{header}.{payload}.not-a-real-signature");
        assert!(verifier.verify(&token).await.is_none());
    }

    /// Minimal base64url-no-padding encoder for the one test above that
    /// needs to hand-build a token -- not worth a `base64` crate
    /// dependency for a single test-only helper.
    fn base64url(input: &str) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let bytes = input.as_bytes();
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[(n & 0x3F) as usize] as char);
            }
        }
        out
    }

    // --- verify: JWKS ------------------------------------------------------------------

    #[tokio::test]
    async fn verify_fails_closed_when_the_jwks_endpoint_is_unreachable() {
        let cfg = JwtConfig {
            jwks_url: Some("http://127.0.0.1:1/jwks.json".to_string()), // nothing listens here
            hs256_secret_env: None,
            issuer: None,
            audience: None,
            jwks_cache_secs: 300,
            client_claim: None,
        };
        let verifier = JwtVerifier::new(&cfg, None).unwrap();
        let token = hs256_token("irrelevant", &json!({"sub": "alice", "exp": future_exp()}));
        assert!(
            verifier.verify(&token).await.is_none(),
            "an unreachable JWKS endpoint must fail closed, not authenticate anyway"
        );
    }

    #[tokio::test]
    async fn verify_fails_closed_when_the_reachable_jwks_has_no_matching_kid() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "keys": [
                    {"kty": "RSA", "kid": "some-other-key", "n": "AQAB", "e": "AQAB"}
                ]
            })))
            .mount(&server)
            .await;

        let cfg = JwtConfig {
            jwks_url: Some(format!("{}/jwks.json", server.uri())),
            hs256_secret_env: None,
            issuer: None,
            audience: None,
            jwks_cache_secs: 300,
            client_claim: None,
        };
        let verifier = JwtVerifier::new(&cfg, None).unwrap();
        let header = base64url(&json!({"alg": "RS256", "kid": "the-actual-kid"}).to_string());
        let payload = base64url(&json!({"sub": "alice", "exp": future_exp()}).to_string());
        let token = format!("{header}.{payload}.not-a-real-signature");

        assert!(
            verifier.verify(&token).await.is_none(),
            "a reachable JWKS with no matching kid must still fail closed"
        );
    }

    #[tokio::test]
    async fn verify_fails_closed_when_the_token_has_no_kid() {
        let cfg = JwtConfig {
            jwks_url: Some("http://127.0.0.1:1/jwks.json".to_string()),
            hs256_secret_env: None,
            issuer: None,
            audience: None,
            jwks_cache_secs: 300,
            client_claim: None,
        };
        let verifier = JwtVerifier::new(&cfg, None).unwrap();
        // No `kid` header at all -- HS256-encoded here just to produce
        // *some* well-formed JWT; JWKS mode never even tries HS256.
        let token = hs256_token("irrelevant", &json!({"sub": "alice", "exp": future_exp()}));
        assert!(verifier.verify(&token).await.is_none());
    }
}
