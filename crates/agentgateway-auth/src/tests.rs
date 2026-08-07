//! JWT policy tests against a real key set and real signatures.
//!
//! Keys are generated at test time, so nothing secret is committed and the
//! signatures are genuinely verified rather than stubbed.

use std::time::{SystemTime, UNIX_EPOCH};

use agentgateway_config::{JwtAuth, JwtSource};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};

use super::*;

const ISSUER: &str = "https://auth.example.com";
const KID: &str = "test-key-1";
const RESOURCE: &str = "https://gateway.example.com/mcp";

/// 2048-bit RSA generation is slow in a debug build and every test wants the
/// same key, so it is generated once for the whole binary.
static PRIMARY: std::sync::OnceLock<(Vec<u8>, Value)> = std::sync::OnceLock::new();

fn keys() -> (EncodingKey, Value) {
    let (der, jwks) = PRIMARY.get_or_init(generate_keys);
    (EncodingKey::from_rsa_der(der), jwks.clone())
}

fn generate_keys() -> (Vec<u8>, Value) {
    use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};

    let mut rng = rand::thread_rng();
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("should generate a key");
    let der = private
        .to_pkcs1_der()
        .expect("should encode the key")
        .as_bytes()
        .to_vec();

    let jwks = json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": KID,
            "n": base64_url(private.n().to_bytes_be()),
            "e": base64_url(private.e().to_bytes_be()),
        }]
    });

    (der, jwks)
}

fn base64_url(bytes: Vec<u8>) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after the epoch")
        .as_secs()
}

/// Sign a token with the shared key, overriding claims as needed.
fn sign(claims: Value) -> String {
    let (encoding, _) = keys();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.into());
    encode(&header, &claims, &encoding).expect("should sign")
}

fn valid_claims() -> Value {
    json!({
        "iss": ISSUER,
        "aud": RESOURCE,
        "sub": "user-1",
        "exp": now() + 3600,
        "scope": "mcp:read mcp:write",
    })
}

/// An authenticator backed by a JWKS file written to a temp directory.
fn authenticator(audiences: &[&str]) -> (JwtAuthenticator, tempfile::TempDir) {
    let (_, jwks) = keys();
    let dir = tempfile::tempdir().expect("should create a temp dir");
    let path = dir.path().join("jwks.json");
    std::fs::write(&path, jwks.to_string()).expect("should write the JWKS");

    let config = JwtAuth {
        issuer: ISSUER.into(),
        audiences: audiences.iter().map(|a| a.to_string()).collect(),
        jwks: JwtSource::File(path.display().to_string()),
    };

    let auth = JwtAuthenticator::new(&config, "test").expect("should build");
    (auth, dir)
}

fn bearer_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::try_from(format!("Bearer {token}")).expect("should be a valid header"),
    );
    headers
}

#[tokio::test]
async fn a_valid_token_is_accepted() {
    let (auth, _dir) = authenticator(&[RESOURCE]);
    let token = auth
        .authenticate(&bearer_headers(&sign(valid_claims())))
        .await
        .expect("a correctly signed token should be accepted");

    assert_eq!(token.subject.as_deref(), Some("user-1"));
    assert!(token.scopes.contains("mcp:read"));
    assert!(token.scopes.contains("mcp:write"));
}

#[tokio::test]
async fn a_token_for_another_audience_is_rejected() {
    // The confused deputy: a caller replays a token minted for some other
    // service and borrows this gateway's privileges. JwtValidator does not
    // check `aud` -- it is this crate's job, and this is the test that says so.
    let (auth, _dir) = authenticator(&[RESOURCE]);
    let mut claims = valid_claims();
    claims["aud"] = json!("https://someone-else.example.com");

    let rejection = auth
        .authenticate(&bearer_headers(&sign(claims)))
        .await
        .expect_err("a token minted for another resource must not be accepted");

    assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
    assert_eq!(rejection.error, Some("invalid_token"));
    assert!(
        !rejection.description.contains("gateway.example.com"),
        "the rejection must not disclose which audiences we accept: {}",
        rejection.description
    );
}

#[tokio::test]
async fn one_matching_audience_in_a_list_is_enough() {
    let (auth, _dir) = authenticator(&[RESOURCE]);
    let mut claims = valid_claims();
    claims["aud"] = json!(["https://other.example.com", RESOURCE]);

    auth.authenticate(&bearer_headers(&sign(claims)))
        .await
        .expect("aud may be an array; one match is enough");
}

#[tokio::test]
async fn an_empty_audience_list_accepts_any_audience() {
    // Documented as a deliberate opt-out for deployments that bind audience
    // upstream -- not an oversight.
    let (auth, _dir) = authenticator(&[]);
    let mut claims = valid_claims();
    claims["aud"] = json!("https://anyone.example.com");

    auth.authenticate(&bearer_headers(&sign(claims)))
        .await
        .expect("an empty audiences list accepts any audience");
}

#[tokio::test]
async fn an_expired_token_is_rejected() {
    let (auth, _dir) = authenticator(&[RESOURCE]);
    let mut claims = valid_claims();
    claims["exp"] = json!(now() - 7200);

    let rejection = auth
        .authenticate(&bearer_headers(&sign(claims)))
        .await
        .expect_err("an expired token must be rejected");

    assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
    assert!(rejection.description.contains("expired"));
}

#[tokio::test]
async fn a_token_from_another_issuer_is_rejected() {
    let (auth, _dir) = authenticator(&[RESOURCE]);
    let mut claims = valid_claims();
    claims["iss"] = json!("https://evil.example.com");

    let rejection = auth
        .authenticate(&bearer_headers(&sign(claims)))
        .await
        .expect_err("an unexpected issuer must be rejected");
    assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_token_signed_by_an_unknown_key_is_rejected() {
    let (auth, _dir) = authenticator(&[RESOURCE]);

    // A different keypair, published nowhere. The signature is well-formed but
    // verifies against nothing we trust.
    use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey};
    let mut rng = rand::thread_rng();
    let other = RsaPrivateKey::new(&mut rng, 2048).expect("should generate a key");
    let der = other
        .to_pkcs1_der()
        .expect("should encode")
        .as_bytes()
        .to_vec();

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.into());
    let forged =
        encode(&header, &valid_claims(), &EncodingKey::from_rsa_der(&der)).expect("should sign");

    let rejection = auth
        .authenticate(&bearer_headers(&forged))
        .await
        .expect_err("a signature from an untrusted key must be rejected");
    assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
    assert!(
        rejection.description.contains("signature"),
        "got: {}",
        rejection.description
    );
}

#[tokio::test]
async fn an_unsigned_token_is_rejected() {
    // `alg: none` is the canonical JWT attack. The algorithm allow-list is
    // checked before any key is loaded, so this dies early.
    let (auth, _dir) = authenticator(&[RESOURCE]);
    let claims = valid_claims();

    use base64::Engine as _;
    let b64 = |v: &Value| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string().as_bytes())
    };
    let unsigned = format!(
        "{}.{}.",
        b64(&json!({"alg": "none", "typ": "JWT", "kid": KID})),
        b64(&claims)
    );

    let rejection = auth
        .authenticate(&bearer_headers(&unsigned))
        .await
        .expect_err("an unsigned token must never be accepted");
    assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_missing_token_is_challenged_without_an_error_code() {
    let (auth, _dir) = authenticator(&[RESOURCE]);
    let rejection = auth
        .authenticate(&HeaderMap::new())
        .await
        .expect_err("a route with jwtAuth requires a token");

    assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        rejection.error, None,
        "RFC 6750: no error code when no credentials were presented"
    );
    assert_eq!(rejection.challenge(), None);
}

#[tokio::test]
async fn a_non_bearer_scheme_is_rejected() {
    let (auth, _dir) = authenticator(&[RESOURCE]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Basic dXNlcjpwYXNz"),
    );

    let rejection = auth
        .authenticate(&headers)
        .await
        .expect_err("only Bearer is accepted");
    assert_eq!(rejection.error, Some("invalid_request"));
}

#[tokio::test]
async fn the_bearer_scheme_is_matched_case_insensitively() {
    // RFC 7235 says the scheme is case-insensitive, and real clients send
    // `bearer`.
    let (auth, _dir) = authenticator(&[RESOURCE]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::try_from(format!("bearer {}", sign(valid_claims())))
            .expect("should be a valid header"),
    );

    auth.authenticate(&headers)
        .await
        .expect("a lowercase scheme is still Bearer");
}

#[test]
fn a_challenge_cannot_be_broken_by_a_quote_in_the_reason() {
    // Validator messages are not ours to trust: an unescaped quote would let
    // one inject header parameters.
    let rejection = AuthRejection::unauthorized("invalid_token", "bad \"kid\" \\ here");
    let challenge = rejection.challenge().expect("should have a challenge");
    assert_eq!(
        challenge.matches('"').count(),
        4,
        "only the four delimiting quotes should survive: {challenge}"
    );
}

#[test]
fn an_unavailable_validator_is_a_503_with_no_challenge() {
    // A 401 here would tell the client to re-authorize, sending a user through
    // a login that fixes nothing and hiding an outage as an auth problem.
    let rejection = AuthRejection {
        status: StatusCode::SERVICE_UNAVAILABLE,
        error: None,
        description: "jwks fetch timed out".into(),
    };
    assert_eq!(rejection.challenge(), None);
}

#[test]
fn a_missing_jwks_file_fails_at_startup() {
    let config = JwtAuth {
        issuer: ISSUER.into(),
        audiences: vec![],
        jwks: JwtSource::File("/nonexistent/jwks.json".into()),
    };
    let err = JwtAuthenticator::new(&config, "binds[0].listeners[0].routes[0]")
        .expect_err("a missing key file must not boot");
    assert!(err.to_string().contains("binds[0]"), "got: {err}");
}

#[test]
fn an_empty_jwks_file_fails_at_startup() {
    // Otherwise every request fails at runtime complaining about an unmatched
    // `kid`, which reads like a client problem rather than a config one.
    let dir = tempfile::tempdir().expect("should create a temp dir");
    let path = dir.path().join("jwks.json");
    std::fs::write(&path, r#"{"keys":[]}"#).expect("should write");

    let config = JwtAuth {
        issuer: ISSUER.into(),
        audiences: vec![],
        jwks: JwtSource::File(path.display().to_string()),
    };
    let err = JwtAuthenticator::new(&config, "test").expect_err("an empty key set must not boot");
    assert!(err.to_string().contains("no keys"), "got: {err}");
}

#[tokio::test]
async fn clock_skew_within_the_leeway_is_tolerated() {
    // A token that expired ten seconds ago is still accepted: the 60s default
    // leeway exists because the gateway's clock and the issuer's are never
    // exactly the same, and rejecting on a few seconds of drift produces
    // failures nobody can reproduce.
    let (auth, _dir) = authenticator(&[RESOURCE]);
    let mut claims = valid_claims();
    claims["exp"] = json!(now() - 10);

    auth.authenticate(&bearer_headers(&sign(claims)))
        .await
        .expect("ten seconds of skew is inside the default leeway");
}
