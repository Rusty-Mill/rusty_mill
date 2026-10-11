//! JWT validator tests against a locally-served JWKS and real signed tokens.
//!
//! Keys are generated at test time, so nothing secret is committed and the
//! signatures are genuinely verified rather than stubbed.

#![cfg(feature = "jwt")]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{Router, routing::get};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rusty_json::{Value, json};
use rusty_mcp::auth::{JwtValidator, TokenError, TokenValidator};

const ISSUER: &str = "https://auth.example.com";
const KID: &str = "test-key-1";
const RESOURCE: &str = "https://mcp.example.com/mcp";

/// An RSA keypair plus the JWKS that publishes its public half.
struct TestKeys {
    encoding: EncodingKey,
    der: Vec<u8>,
    jwks: Value,
}

/// Cached DER + JWKS for the primary key.
///
/// 2048-bit RSA generation is slow in a debug build, and every test needs the
/// same key, so it is generated once for the binary rather than per test.
static PRIMARY: std::sync::OnceLock<(Vec<u8>, Value)> = std::sync::OnceLock::new();

/// The shared key every test signs with.
fn keys() -> TestKeys {
    let (der, jwks) = PRIMARY.get_or_init(|| {
        let generated = generate_keys();
        (generated.der, generated.jwks)
    });

    TestKeys {
        encoding: EncodingKey::from_rsa_der(der),
        jwks: jwks.clone(),
        der: der.clone(),
    }
}

fn generate_keys() -> TestKeys {
    use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};

    // 2048 bits: the smallest size a real authorization server would use.
    let mut rng = rand::thread_rng();
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("generate key");

    let der = private
        .to_pkcs1_der()
        .expect("encode key")
        .as_bytes()
        .to_vec();
    let encoding = EncodingKey::from_rsa_der(&der);

    let n = base64_url(private.n().to_bytes_be());
    let e = base64_url(private.e().to_bytes_be());

    TestKeys {
        encoding,
        der,
        jwks: json!({
            "keys": [{
                "kty": "RSA",
                "use": "sig",
                "alg": "RS256",
                "kid": KID,
                "n": n,
                "e": e,
            }]
        }),
    }
}

fn base64_url(bytes: Vec<u8>) -> String {
    rusty_base64::encode_url_safe_no_pad(&bytes)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

fn sign(keys: &TestKeys, claims: Value) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.to_string());
    encode(&header, &claims, &keys.encoding).expect("sign token")
}

/// Serve the JWKS on a local port and return its URI.
async fn serve_jwks(jwks: Value) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    let app = Router::new().route(
        "/jwks.json",
        get(move || {
            let jwks = jwks.clone();
            async move { axum::Json(jwks) }
        }),
    );

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    format!("http://{addr}/jwks.json")
}

async fn validator_for(keys: &TestKeys) -> JwtValidator {
    let jwks_uri = serve_jwks(keys.jwks.clone()).await;
    JwtValidator::builder(ISSUER, jwks_uri)
        .build()
        .expect("build validator")
}

#[tokio::test]
async fn accepts_a_correctly_signed_token() {
    let keys = keys();
    let validator = validator_for(&keys).await;

    let token = sign(
        &keys,
        json!({
            "iss": ISSUER,
            "sub": "user-1",
            "aud": RESOURCE,
            "exp": now() + 600,
            "scope": "mcp:read mcp:write",
            "client_id": "cli-9",
        }),
    );

    let verified = validator.validate(&token).await.expect("should validate");

    assert_eq!(verified.subject.as_deref(), Some("user-1"));
    assert_eq!(verified.client_id.as_deref(), Some("cli-9"));
    assert_eq!(verified.audiences, vec![RESOURCE]);
    assert!(verified.scopes.contains("mcp:read"));
    assert!(verified.scopes.contains("mcp:write"));
    // The layer, not the validator, decides whether the audience is ours.
    assert!(!verified.audience_verified);
}

#[tokio::test]
async fn rejects_an_expired_token() {
    let keys = keys();
    let validator = validator_for(&keys).await;

    let token = sign(
        &keys,
        json!({
            "iss": ISSUER,
            "aud": RESOURCE,
            // Well past the default 60s leeway.
            "exp": now() - 3600,
        }),
    );

    assert!(matches!(
        validator.validate(&token).await,
        Err(TokenError::Expired)
    ));
}

#[tokio::test]
async fn rejects_a_token_from_the_wrong_issuer() {
    let keys = keys();
    let validator = validator_for(&keys).await;

    let token = sign(
        &keys,
        json!({
            "iss": "https://evil.example.com",
            "aud": RESOURCE,
            "exp": now() + 600,
        }),
    );

    let err = validator.validate(&token).await.expect_err("should reject");
    assert!(
        matches!(err, TokenError::Invalid(ref m) if m.contains("issuer")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn rejects_a_token_signed_by_an_unknown_key() {
    let keys = keys();
    let validator = validator_for(&keys).await;

    // Sign with a *different* keypair, published nowhere. This one really is
    // generated fresh — sharing it would defeat the test.
    let attacker = generate_keys();
    let token = sign(
        &attacker,
        json!({
            "iss": ISSUER,
            "aud": RESOURCE,
            "exp": now() + 600,
        }),
    );

    // Same `kid`, wrong key: the signature check is what catches this.
    assert!(matches!(
        validator.validate(&token).await,
        Err(TokenError::Invalid(_))
    ));
}

#[tokio::test]
async fn rejects_an_unsigned_token() {
    let keys = keys();
    let validator = validator_for(&keys).await;

    // `alg: none` with an empty signature — the classic downgrade attempt.
    let header = rusty_base64::encode_url_safe_no_pad(
        json!({"alg": "none", "kid": KID})
            .to_json_string()
            .as_bytes(),
    );
    let payload = rusty_base64::encode_url_safe_no_pad(
        json!({"iss": ISSUER, "exp": now() + 600})
            .to_json_string()
            .as_bytes(),
    );
    let token = format!("{header}.{payload}.");

    assert!(matches!(
        validator.validate(&token).await,
        Err(TokenError::Invalid(_))
    ));
}

#[tokio::test]
async fn rejects_a_token_with_no_kid() {
    let keys = keys();
    let validator = validator_for(&keys).await;

    let mut header = Header::new(Algorithm::RS256);
    header.kid = None;
    let token = encode(
        &header,
        &json!({"iss": ISSUER, "aud": RESOURCE, "exp": now() + 600}),
        &keys.encoding,
    )
    .expect("sign");

    let err = validator.validate(&token).await.expect_err("should reject");
    assert!(
        matches!(err, TokenError::Invalid(ref m) if m.contains("kid")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn an_unreachable_jwks_is_unavailable_not_invalid() {
    // A 503 keeps the client's good token from being blamed for a server-side
    // outage — the layer turns `Unavailable` into 503, not 401.
    let validator = JwtValidator::builder(ISSUER, "http://127.0.0.1:1/jwks.json")
        .with_request_timeout(Duration::from_millis(200))
        .build()
        .expect("build");

    let keys = keys();
    let token = sign(
        &keys,
        json!({"iss": ISSUER, "aud": RESOURCE, "exp": now() + 600}),
    );

    assert!(matches!(
        validator.validate(&token).await,
        Err(TokenError::Unavailable(_))
    ));
}

/// A token that parses far enough to need a key from the JWKS, without paying
/// for a real signature: the fetch fails (or is refused) before it is checked.
fn token_needing_a_key() -> String {
    let header = json!({"alg": "RS256", "typ": "JWT", "kid": KID}).to_json_string();
    let payload = json!({"iss": ISSUER, "aud": RESOURCE, "exp": now() + 600}).to_json_string();
    format!(
        "{}.{}.c2ln",
        base64_url(header.into_bytes()),
        base64_url(payload.into_bytes())
    )
}

/// Serve `body` with `status` at `/jwks.json` and return the URI.
async fn serve_raw(status: http::StatusCode, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = Router::new().route("/jwks.json", get(move || async move { (status, body) }));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}/jwks.json")
}

async fn unavailable_reason(validator: &JwtValidator) -> String {
    match validator.validate(&token_needing_a_key()).await {
        Err(TokenError::Unavailable(why)) => why,
        Err(other) => panic!("expected Unavailable, got {other}"),
        Ok(_) => panic!("a token must not validate without its key"),
    }
}

#[tokio::test]
async fn a_failing_jwks_endpoint_is_unavailable_and_says_why() {
    let uri = serve_raw(http::StatusCode::SERVICE_UNAVAILABLE, "down").await;
    let validator = JwtValidator::builder(ISSUER, uri).build().expect("build");

    let why = unavailable_reason(&validator).await;

    assert!(why.contains("503"), "{why}");
}

#[tokio::test]
async fn a_malformed_jwks_is_unavailable_not_invalid() {
    let uri = serve_raw(http::StatusCode::OK, "this is not json").await;
    let validator = JwtValidator::builder(ISSUER, uri).build().expect("build");

    let why = unavailable_reason(&validator).await;

    assert!(why.contains("malformed JWKS"), "{why}");
}

#[tokio::test]
async fn a_malformed_jwks_uri_is_unavailable_not_a_panic() {
    let validator = JwtValidator::builder(ISSUER, "not a url")
        .build()
        .expect("build");

    let why = unavailable_reason(&validator).await;

    assert!(why.contains("JWKS fetch failed"), "{why}");
}

#[tokio::test]
async fn a_jwks_server_that_never_answers_is_cut_off_by_the_request_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    // Accept connections and hold them open without ever replying.
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let validator = JwtValidator::builder(ISSUER, format!("http://{addr}/jwks.json"))
        .with_request_timeout(Duration::from_millis(200))
        .build()
        .expect("build");

    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        validator.validate(&token_needing_a_key()),
    )
    .await
    .expect("the request timeout should fire long before this outer one");

    assert!(matches!(outcome, Err(TokenError::Unavailable(_))));
}

#[tokio::test]
async fn reads_an_array_audience_and_array_scopes() {
    let keys = keys();
    let jwks_uri = serve_jwks(keys.jwks.clone()).await;
    let validator = JwtValidator::builder(ISSUER, jwks_uri)
        .with_scope_claim("scp")
        .build()
        .expect("build");

    let token = sign(
        &keys,
        json!({
            "iss": ISSUER,
            "aud": ["https://other.example.com", RESOURCE],
            "exp": now() + 600,
            "scp": ["mcp:read", "mcp:write"],
        }),
    );

    let verified = validator.validate(&token).await.expect("validates");
    assert!(verified.audiences.contains(&RESOURCE.to_string()));
    assert!(verified.scopes.contains("mcp:write"));
}
