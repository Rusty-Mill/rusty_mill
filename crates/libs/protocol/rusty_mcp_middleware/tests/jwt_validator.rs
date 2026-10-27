//! JWT validator tests against a locally-served JWKS and real signed tokens.
//!
//! Keys are generated at test time, so nothing secret is committed and the
//! signatures are genuinely verified rather than stubbed.

#![cfg(feature = "jwt")]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rusty_http::StatusCode;
use rusty_json::{Value, json};
use rusty_mcp_middleware::auth::{JwtAlgorithm, JwtValidator, TokenError, TokenValidator};
use rusty_oauth::jwt::{encode_hs256, es256};

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
    serve_raw(StatusCode::OK, jwks.to_json_string()).await
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
///
/// A blocking `rusty_serve` server on a thread of its own, so the mock shares
/// no runtime with the validator under test, which fetches on the test's.
async fn serve_raw(status: StatusCode, body: impl Into<Vec<u8>>) -> String {
    let body = body.into();
    let handler = move |request: &rusty_serve::Request<'_>| {
        if request.target == "/jwks.json" {
            rusty_serve::Response::json(status, body.clone())
        } else {
            rusty_serve::Response::json(StatusCode::NOT_FOUND, Vec::new())
        }
    };
    let server = rusty_serve::Server::bind_shared("127.0.0.1:0".parse().expect("addr"), handler)
        .expect("bind");
    let addr = server.local_addr().expect("addr");
    std::thread::spawn(move || {
        let _ = server.run();
    });
    format!("http://{addr}/jwks.json")
}

fn fetch_test_claims() -> Value {
    json!({"iss": ISSUER, "sub": "user-1", "aud": RESOURCE, "exp": now() + 600})
}

/// A raw HTTP/1.1 server: every connection gets `reply` verbatim and is then
/// closed. Replies that advertise keep-alive let a client believe the
/// connection can be reused after the server has already hung up.
async fn serve_raw_replies(reply: String) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let reply = reply.clone();
            tokio::spawn(async move {
                let mut request = [0u8; 4096];
                let _ = socket.read(&mut request).await;
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
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

/// The test key's JWKS plus unrelated filler keys, until the document is at
/// least `target` bytes: a stand-in for a provider with a large key set.
fn jwks_padded_to(keys: &TestKeys, target: usize) -> Value {
    let mut all = keys.jwks["keys"].as_array().expect("keys").clone();
    let mut size = keys.jwks.to_json_string().len();
    let filler = "A".repeat(3000);
    let mut n = 0;
    while size < target {
        let entry = json!({"kty": "oct", "kid": format!("filler-{n}"), "k": filler.clone()});
        size += entry.to_json_string().len() + 1;
        all.push(entry);
        n += 1;
    }
    json!({"keys": Value::Array(all)})
}

#[tokio::test]
async fn a_jwks_larger_than_rusty_requests_default_cap_is_still_read() {
    // Bot Framework's key set is about 910 KB today; this one is 1.5 MiB, over
    // the 1 MiB default, and the real key inside it must still be usable.
    let keys = keys();
    let jwks = jwks_padded_to(&keys, 3 << 19);
    assert!(jwks.to_json_string().len() > 1 << 20);
    let uri = serve_jwks(jwks).await;
    let validator = JwtValidator::builder(ISSUER, uri).build().expect("build");

    let verified = validator
        .validate(&sign(&keys, valid_claims()))
        .await
        .expect("a valid token must validate against a large key set");

    assert_eq!(verified.subject.as_deref(), Some("user-1"));
}

#[tokio::test]
async fn a_jwks_over_the_validators_own_cap_is_unavailable() {
    // The cap is 4 MiB, so this is refused, not read without bound.
    let keys = keys();
    let uri = serve_jwks(jwks_padded_to(&keys, (4 << 20) + 1024)).await;
    let validator = JwtValidator::builder(ISSUER, uri).build().expect("build");

    let why = unavailable_reason(&validator).await;

    assert!(why.contains("maximum allowed size"), "{why}");
}

#[tokio::test]
async fn a_failing_jwks_endpoint_is_unavailable_and_says_why() {
    let uri = serve_raw(StatusCode::SERVICE_UNAVAILABLE, "down").await;
    let validator = JwtValidator::builder(ISSUER, uri).build().expect("build");

    let why = unavailable_reason(&validator).await;

    assert!(why.contains("503"), "{why}");
}

#[tokio::test]
async fn a_jwks_server_that_closes_its_keepalive_connection_is_still_reachable() {
    // The server says keep-alive and then hangs up, as an idle timeout would.
    // Reusing that connection fails with no retry, and a failed fetch would
    // also block the next one for the refetch interval, so fetches must not
    // reuse connections.
    let keys = keys();
    let body = keys.jwks.to_json_string();
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
        body.len()
    );
    let validator = JwtValidator::builder(ISSUER, serve_raw_replies(reply).await)
        .with_jwks_ttl(Duration::ZERO)
        .with_min_refetch_interval(Duration::ZERO)
        .build()
        .expect("build");

    for round in 1..=3 {
        let outcome = validator.validate(&sign(&keys, fetch_test_claims())).await;
        assert!(outcome.is_ok(), "fetch {round} failed: {outcome:?}");
        // Give the server's close time to reach the client.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_jwks_response_preceded_by_an_informational_one_is_read() {
    // `103 Early Hints` before the final response is legal HTTP/1.1; the final
    // `200` is the answer, and arriving in the same packet must not lose it.
    let keys = keys();
    let body = keys.jwks.to_json_string();
    let reply = format!(
        "HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let validator = JwtValidator::builder(ISSUER, serve_raw_replies(reply).await)
        .build()
        .expect("build");

    let outcome = validator.validate(&sign(&keys, fetch_test_claims())).await;

    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
async fn a_malformed_jwks_is_unavailable_not_invalid() {
    let uri = serve_raw(StatusCode::OK, "this is not json").await;
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

fn valid_claims() -> Value {
    json!({"iss": ISSUER, "sub": "user-1", "aud": RESOURCE, "exp": now() + 600})
}

/// The `Invalid` reason for a token the validator must refuse.
async fn rejection(validator: &JwtValidator, token: &str) -> String {
    match validator.validate(token).await {
        Err(TokenError::Invalid(why)) => why,
        Err(other) => panic!("expected Invalid, got {other}"),
        Ok(_) => panic!("the token must be rejected"),
    }
}

/// The three segments of a compact token.
fn segments(token: &str) -> (String, String, String) {
    let mut parts = token.split('.');
    let mut next = || parts.next().expect("a three-part token").to_string();
    (next(), next(), next())
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

#[tokio::test]
async fn accepts_an_es256_token() {
    // A fixed P-256 test key, used only here. `rusty_oauth` can sign ES256,
    // so this path needs no second JWT library.
    let private = es256::EcPrivateKey::from_bytes(&hex(
        "67718fec6a6b21b412a5c5306286f1ee30e32498fd6c61b66f57d0ad1d7c0738",
    ))
    .expect("private key");
    let (x, y) = private.public_key().to_affine_coordinates();
    let jwks = json!({"keys": [{
        "kty": "EC", "crv": "P-256", "kid": "ec-1", "use": "sig", "alg": "ES256",
        "x": base64_url(x), "y": base64_url(y),
    }]});
    let validator = JwtValidator::builder(ISSUER, serve_jwks(jwks).await)
        .build()
        .expect("build");

    let token = es256::encode_es256(&valid_claims(), &private, &[("kid", "ec-1")]);

    let verified = validator.validate(&token).await.expect("should validate");
    assert_eq!(verified.subject.as_deref(), Some("user-1"));
}

#[tokio::test]
async fn rejects_an_hs256_token_keyed_with_the_rsa_public_key() {
    // The classic algorithm-confusion forgery: an HMAC token "signed" with the
    // public key as the secret. The algorithm is refused before any key is read.
    let keys = keys();
    let validator = validator_for(&keys).await;
    let modulus = keys.jwks["keys"][0]["n"].as_str().expect("modulus");

    let token = encode_hs256(&valid_claims(), modulus.as_bytes(), &[("kid", KID)]);

    assert!(rejection(&validator, &token).await.contains("not accepted"));
}

#[tokio::test]
async fn rejects_an_algorithm_the_validator_was_not_given() {
    let keys = keys();
    let validator = JwtValidator::builder(ISSUER, serve_jwks(keys.jwks.clone()).await)
        .with_algorithms([JwtAlgorithm::Es256])
        .build()
        .expect("build");

    let token = sign(&keys, valid_claims());

    assert!(rejection(&validator, &token).await.contains("not accepted"));
}

#[tokio::test]
async fn rejects_an_rsa_key_presented_for_an_es256_token() {
    // A genuine RS256 signature under a header that now claims ES256: the RSA
    // key is never reinterpreted for the other algorithm.
    let keys = keys();
    let validator = validator_for(&keys).await;
    let (_, payload, signature) = segments(&sign(&keys, valid_claims()));
    let header = json!({"alg": "ES256", "typ": "JWT", "kid": KID}).to_json_string();

    let forged = format!("{}.{payload}.{signature}", base64_url(header.into_bytes()));

    assert!(
        rejection(&validator, &forged)
            .await
            .contains("unusable signing key")
    );
}

#[tokio::test]
async fn rejects_a_tampered_payload() {
    let keys = keys();
    let validator = validator_for(&keys).await;
    let (header, _, signature) = segments(&sign(&keys, valid_claims()));
    let forged_claims = json!({"iss": ISSUER, "sub": "admin", "aud": RESOURCE, "exp": now() + 600});

    let forged = format!(
        "{header}.{}.{signature}",
        base64_url(forged_claims.to_json_string().into_bytes())
    );

    assert!(
        rejection(&validator, &forged)
            .await
            .contains("signature is invalid")
    );
}

#[tokio::test]
async fn rejects_a_garbled_or_truncated_signature() {
    let keys = keys();
    let validator = validator_for(&keys).await;
    let (header, payload, signature) = segments(&sign(&keys, valid_claims()));

    // Well-formed signatures of the wrong value or length fail verification.
    let genuine = rusty_base64::decode_url_safe(&signature).expect("a signature");
    let zeroed = base64_url(vec![0; 256]);
    let one_byte_short = base64_url(genuine[..genuine.len() - 1].to_vec());
    for bad in [zeroed, one_byte_short, String::new()] {
        let token = format!("{header}.{payload}.{bad}");
        let why = rejection(&validator, &token).await;
        assert!(why.contains("signature is invalid"), "{why}");
    }

    // A tail whose unused padding bits are not zero is non-canonical base64
    // ("AB" is 12 bits: one byte, then four bits that are `0001`). The strict
    // decoder refuses it before any signature check. (Cutting the real
    // signature would only sometimes land on such a tail, so use a fixed one.)
    let non_canonical = format!("{header}.{payload}.AB");
    assert!(
        rejection(&validator, &non_canonical)
            .await
            .contains("malformed token")
    );
}

#[tokio::test]
async fn rejects_a_token_that_is_not_yet_valid() {
    let keys = keys();
    let validator = validator_for(&keys).await;
    let claims = json!({
        "iss": ISSUER, "aud": RESOURCE, "exp": now() + 7200, "nbf": now() + 3600,
    });

    let token = sign(&keys, claims);

    assert!(
        rejection(&validator, &token)
            .await
            .contains("not yet valid")
    );
}

#[tokio::test]
async fn accepts_a_token_that_expired_inside_the_leeway() {
    let keys = keys();
    let validator = validator_for(&keys).await;
    let claims = json!({"iss": ISSUER, "aud": RESOURCE, "exp": now() - 30});

    assert!(validator.validate(&sign(&keys, claims)).await.is_ok());
}

#[tokio::test]
async fn rejects_a_token_with_no_expiry() {
    let keys = keys();
    let validator = validator_for(&keys).await;
    let claims = json!({"iss": ISSUER, "aud": RESOURCE});

    assert!(
        rejection(&validator, &sign(&keys, claims))
            .await
            .contains("validation failed")
    );
}

#[tokio::test]
async fn rejects_malformed_tokens_without_panicking() {
    let keys = keys();
    let validator = validator_for(&keys).await;

    for token in ["", "abc", "a.b", "a.b.c.d", "....", "!!!.!!!.!!!"] {
        assert!(
            rejection(&validator, token)
                .await
                .contains("malformed token"),
            "{token:?}"
        );
    }
}

#[tokio::test]
async fn rejects_a_header_with_no_algorithm_or_no_kid() {
    let keys = keys();
    let validator = validator_for(&keys).await;
    // `e30` is `{}`: a well-formed token with an empty header and payload.
    assert!(rejection(&validator, "e30.e30.").await.contains("no `alg`"));

    let header = base64_url(json!({"alg": "RS256"}).to_json_string().into_bytes());
    let payload = base64_url(valid_claims().to_json_string().into_bytes());
    let token = format!("{header}.{payload}.c2ln");
    assert!(rejection(&validator, &token).await.contains("no `kid`"));
}

#[tokio::test]
async fn rejects_deeply_nested_json_in_a_token() {
    // Header and payload are read before the caller is authenticated, so a
    // pathological nesting depth must be refused, not recursed into.
    let keys = keys();
    let validator = validator_for(&keys).await;
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    let nested = base64_url(deep.into_bytes());
    let empty = base64_url(b"{}".to_vec());

    for token in [
        format!("{nested}.{empty}.c2ln"),
        format!("{empty}.{nested}.c2ln"),
    ] {
        assert!(
            rejection(&validator, &token)
                .await
                .contains("malformed token")
        );
    }
}

#[tokio::test]
async fn a_duplicated_kid_in_the_jwks_matches_no_key() {
    // Two keys claiming one `kid` is ambiguous, and picking one would be the
    // wrong way to fail a security-relevant lookup.
    let keys = keys();
    let key = keys.jwks["keys"][0].clone();
    let jwks = json!({"keys": [key.clone(), key]});
    let validator = JwtValidator::builder(ISSUER, serve_jwks(jwks).await)
        .build()
        .expect("build");

    let token = sign(&keys, valid_claims());

    assert!(
        rejection(&validator, &token)
            .await
            .contains("no signing key matches")
    );
}

#[tokio::test]
async fn refuses_rsa_keys_outside_the_size_bounds_without_using_them() {
    // 8192 bits would make every verification expensive; 1024 is too weak.
    for modulus_bytes in [1024, 128] {
        let jwks = json!({"keys": [{
            "kty": "RSA", "kid": KID, "n": base64_url(vec![0xFF; modulus_bytes]), "e": "AQAB",
        }]});
        let validator = JwtValidator::builder(ISSUER, serve_jwks(jwks).await)
            .build()
            .expect("build");

        let why = rejection(&validator, &token_needing_a_key()).await;

        assert!(
            why.contains("unusable signing key"),
            "{modulus_bytes}: {why}"
        );
    }
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

#[tokio::test]
async fn signed_numeric_dates_preserve_valid_forms_and_fail_closed() {
    let keys = keys();
    let validator = validator_for(&keys).await;
    for exp in [
        json!(now() + 600),
        json!(now() as f64 + 600.5),
        json!(4102444800.0),
        json!(u64::MAX),
    ] {
        for nbf in [
            None,
            Some(json!(now() - 60)),
            Some(json!(now() as f64 - 60.5)),
        ] {
            let mut claims = json!({"iss": ISSUER, "exp": exp.clone()});
            if let Some(nbf) = nbf {
                claims["nbf"] = nbf;
            }
            assert!(
                validator
                    .validate(&sign(&keys, claims.clone()))
                    .await
                    .is_ok(),
                "{claims:?}"
            );
        }
    }
    for nbf in [json!(4102444800.5), json!(u64::MAX), json!(now() + 3600)] {
        let claims = json!({"iss": ISSUER, "exp": u64::MAX, "nbf": nbf});
        assert!(
            rejection(&validator, &sign(&keys, claims))
                .await
                .contains("not yet valid")
        );
    }
    for field in ["exp", "nbf"] {
        for invalid in [
            json!("4102444800"),
            json!(null),
            json!(-1),
            json!(-0.5),
            json!(true),
            json!([]),
            json!({}),
            json!(18446744073709551616.0),
            json!(1e100),
        ] {
            let mut claims = json!({"iss": ISSUER, "exp": now() + 600});
            claims[field] = invalid;
            assert!(
                rejection(&validator, &sign(&keys, claims))
                    .await
                    .contains("invalid NumericDate")
            );
        }
    }
    let claims = json!({"iss": ISSUER, "exp": now() as f64 - 3600.5});
    assert!(matches!(
        validator.validate(&sign(&keys, claims)).await,
        Err(TokenError::Expired)
    ));
}

#[tokio::test]
async fn verifies_independent_openssl_es256_and_rejects_bad_scalars_and_keys() {
    // Uses the key from rusty_oauth::jwt::es256::tests, but a new signature
    // because MCP requires kid. Generated independently with OpenSSL:
    // openssl dgst -sha256 -sign test-key.pem -out signature.der signing-input
    // Decode DER r/s, left-pad each to 32 bytes, concatenate, base64url encode.
    // Header: {"alg":"ES256","typ":"JWT","kid":"ec-1"}
    // Payload: {"iss":"https://auth.example.com","sub":"openssl-user","exp":4102444800}
    const TOKEN: &str = "eyJhbGciOiJFUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6ImVjLTEifQ.eyJpc3MiOiJodHRwczovL2F1dGguZXhhbXBsZS5jb20iLCJzdWIiOiJvcGVuc3NsLXVzZXIiLCJleHAiOjQxMDI0NDQ4MDB9.ui0VYgAR1_dka872KHYMl0foGaB5iJ7MD7PU0bgaHUVf6wM3fxX60HeSDidj9RRZjupnbmFgHz3rxoxPKx4KeA";
    let x = hex("9958e30d1b1ca2943fb08c191400beab172729085e843cf130422d686bf81a7b");
    let y = hex("a7613a86bac66693dd6adead383e9e1f0407424dc7281049bce06c3fefa91e6f");
    let jwks = json!({"keys": [{"kty": "EC", "crv": "P-256", "kid": "ec-1",
        "x": base64_url(x), "y": base64_url(y)}]});
    let validator = JwtValidator::builder(ISSUER, serve_jwks(jwks.clone()).await)
        .build()
        .expect("build validator");
    assert_eq!(
        validator
            .validate(TOKEN)
            .await
            .expect("OpenSSL signature")
            .subject
            .as_deref(),
        Some("openssl-user")
    );
    let (header, payload, signature) = segments(TOKEN);
    let signature = rusty_base64::decode_url_safe(&signature).expect("signature base64");
    let order = hex("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551");
    for offset in [0, 32] {
        for scalar in [vec![0; 32], order.clone(), vec![0xff; 32]] {
            let mut bad = signature.clone();
            bad[offset..offset + 32].copy_from_slice(&scalar);
            let token = format!("{header}.{payload}.{}", base64_url(bad));
            assert!(
                rejection(&validator, &token)
                    .await
                    .contains("signature is invalid")
            );
        }
    }
    for bad in [signature[..63].to_vec(), vec![0; 65]] {
        let token = format!("{header}.{payload}.{}", base64_url(bad));
        assert!(
            rejection(&validator, &token)
                .await
                .contains("signature is invalid")
        );
    }
    for coordinate in ["x", "y"] {
        for bad in [vec![0; 32], vec![0xff; 32], vec![0; 31], vec![0; 33]] {
            let mut malformed = jwks.clone();
            malformed["keys"][0][coordinate] = json!(base64_url(bad));
            let validator = JwtValidator::builder(ISSUER, serve_jwks(malformed).await)
                .build()
                .expect("build validator");
            assert!(
                rejection(&validator, TOKEN)
                    .await
                    .contains("unusable signing key")
            );
        }
    }
}
