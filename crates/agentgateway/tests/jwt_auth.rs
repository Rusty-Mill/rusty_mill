//! End-to-end tests for the `jwtAuth` route policy.
//!
//! These drive real HTTP against a running gateway with a real signed token,
//! so they cover the wiring the unit tests in `agentgateway-auth` cannot: that
//! the policy is actually consulted, that it runs *after* the CORS preflight
//! branch, and that a rejection carries a `WWW-Authenticate` challenge.

use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const ISSUER: &str = "https://auth.example.com";
const KID: &str = "gateway-test-key";
const RESOURCE: &str = "https://gateway.example.com/mcp";

static PRIMARY: std::sync::OnceLock<(Vec<u8>, Value)> = std::sync::OnceLock::new();

fn keys() -> (EncodingKey, Value) {
    let (der, jwks) = PRIMARY.get_or_init(|| {
        use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};
        use base64::Engine as _;

        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("should generate a key");
        let der = private
            .to_pkcs1_der()
            .expect("should encode")
            .as_bytes()
            .to_vec();
        let b64 = |bytes: Vec<u8>| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);

        let jwks = json!({
            "keys": [{
                "kty": "RSA", "use": "sig", "alg": "RS256", "kid": KID,
                "n": b64(private.n().to_bytes_be()),
                "e": b64(private.e().to_bytes_be()),
            }]
        });
        (der, jwks)
    });
    (EncodingKey::from_rsa_der(der), jwks.clone())
}

fn sign(claims: Value) -> String {
    let (encoding, _) = keys();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.into());
    encode(&header, &claims, &encoding).expect("should sign")
}

fn valid_claims() -> Value {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after the epoch")
        .as_secs();
    json!({ "iss": ISSUER, "aud": RESOURCE, "sub": "user-1", "exp": now + 3600 })
}

fn mock_server() -> String {
    let mut path = std::env::current_exe().expect("test binary should have a path");
    path.pop();
    path.pop();
    path.push("examples");
    path.push("mock_mcp_server");
    path.display().to_string()
}

async fn free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind");
    listener.local_addr().expect("should have an addr").port()
}

const CONFIG: &str = r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: guarded
            matches:
              - path:
                  pathPrefix: /mcp
            policies:
              cors:
                allowOrigins: ["*"]
                allowHeaders: [authorization, content-type]
              jwtAuth:
                issuer: https://auth.example.com
                audiences: ["https://gateway.example.com/mcp"]
                jwks:
                  file: "{jwks}"
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
"#;

/// Boot a guarded gateway; returns its base URL and a shutdown handle.
async fn start() -> (String, CancellationToken, tempfile::TempDir) {
    let (_, jwks) = keys();
    let dir = tempfile::tempdir().expect("should create a temp dir");
    let jwks_path = dir.path().join("jwks.json");
    std::fs::write(&jwks_path, jwks.to_string()).expect("should write the JWKS");

    let port = free_port().await;
    let yaml = CONFIG
        .replace("{port}", &port.to_string())
        .replace("{server}", &mock_server())
        .replace("{jwks}", &jwks_path.display().to_string());

    let config = Config::from_yaml(&yaml).expect("config should parse");
    config.validate().expect("config should validate");
    let gateway = Gateway::build(&config).await.expect("gateway should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    // The returned future only joins the accept loops; the tests drive the
    // gateway over HTTP and cancel it directly, so it is deliberately dropped.
    let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    (format!("http://127.0.0.1:{port}/mcp"), shutdown, dir)
}

/// A minimal MCP `initialize`, which is the first thing any client sends.
fn initialize_body() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1"}
        }
    })
}

async fn post(url: &str, token: Option<&str>) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&initialize_body());
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    request.send().await.expect("request should reach the gateway")
}

#[tokio::test]
async fn a_valid_token_reaches_the_backend() {
    let (url, shutdown, _dir) = start().await;

    let response = post(&url, Some(&sign(valid_claims()))).await;
    assert!(
        response.status().is_success(),
        "a valid token should be let through, got {}",
        response.status()
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_request_with_no_token_is_challenged() {
    let (url, shutdown, _dir) = start().await;

    let response = post(&url, None).await;
    assert_eq!(response.status(), 401);
    assert_eq!(
        response
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer"),
        "a 401 without a challenge leaves the client no way to learn it should authenticate"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_token_for_another_audience_never_reaches_the_backend() {
    let (url, shutdown, _dir) = start().await;

    let mut claims = valid_claims();
    claims["aud"] = json!("https://someone-else.example.com");

    let response = post(&url, Some(&sign(claims))).await;
    assert_eq!(
        response.status(),
        401,
        "a token minted for another resource must be refused at the gateway"
    );
    let challenge = response
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        challenge.contains("invalid_token"),
        "got challenge: {challenge}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_garbage_token_is_rejected() {
    let (url, shutdown, _dir) = start().await;

    let response = post(&url, Some("not-a-jwt")).await;
    assert_eq!(response.status(), 401);

    shutdown.cancel();
}

#[tokio::test]
async fn a_cors_preflight_is_answered_without_a_token() {
    // Browsers never send `Authorization` on a preflight. If auth ran before
    // the preflight branch, every cross-origin call would fail here and the
    // real request would never be sent -- so this asserts the ordering.
    let (url, shutdown, _dir) = start().await;

    let response = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, &url)
        .header("origin", "https://app.example.com")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "authorization")
        .send()
        .await
        .expect("preflight should reach the gateway");

    assert_eq!(
        response.status(),
        204,
        "an unauthenticated preflight must still be answered"
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("*")
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_rejection_still_carries_cors_headers() {
    // Without these the browser reports an opaque network error instead of the
    // 401, and the caller cannot tell "log in" from "the gateway is down".
    let (url, shutdown, _dir) = start().await;

    let response = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("origin", "https://app.example.com")
        .json(&initialize_body())
        .send()
        .await
        .expect("request should reach the gateway");

    assert_eq!(response.status(), 401);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("*"),
        "a 401 the browser cannot read is a 401 nobody can act on"
    );

    shutdown.cancel();
}
