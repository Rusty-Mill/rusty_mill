//! End-to-end tests for a target's `identity` block (Let Em Loose #1376).
//!
//! The unit tests in `agentgateway-mcp` cover how a call is stamped. These
//! cover the wiring: that the claims come from a token this gateway verified,
//! that a forged actor in the call never reaches the upstream, and that a call
//! without a token is refused rather than sent unstamped.

use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use agentgateway_config::Config;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rusty_tokio::sync::CancellationToken;
use serde_json::{Value, json};

const ISSUER: &str = "https://auth.example.com";
const KID: &str = "gateway-test-key";
const RESOURCE: &str = "https://gateway.example.com/mcp";

static PRIMARY: std::sync::OnceLock<(Vec<u8>, Value)> = std::sync::OnceLock::new();

fn keys() -> (EncodingKey, Value) {
    let (der, jwks) = PRIMARY.get_or_init(|| {
        use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};

        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("should generate a key");
        let der = private
            .to_pkcs1_der()
            .expect("should encode")
            .as_bytes()
            .to_vec();
        let b64 = |bytes: Vec<u8>| rusty_base64::encode_url_safe_no_pad(&bytes);

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

/// A token carrying `extra` on top of the claims the route requires.
fn token(extra: Value) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after the epoch")
        .as_secs();
    let mut claims = json!({ "iss": ISSUER, "aud": RESOURCE, "exp": now + 3600 });
    if let (Some(claims), Value::Object(extra)) = (claims.as_object_mut(), extra) {
        claims.extend(extra);
    }

    let (encoding, _) = keys();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.into());
    encode(&header, &claims, &encoding).expect("should sign")
}

fn mock_server() -> String {
    let mut path = std::env::current_exe().expect("test binary should have a path");
    path.pop(); // deps/
    path.pop(); // <profile>/
    path.push("examples");
    // Windows names the built example `mock_mcp_server.exe`; `EXE_SUFFIX` is
    // empty everywhere else.
    path.push(format!("mock_mcp_server{}", std::env::consts::EXE_SUFFIX));
    assert!(path.exists(), "fixture not built at {}", path.display());
    // The caller splices this into a double-quoted YAML scalar (`cmd:
    // "{server}"`), so it goes through `yaml_path`.
    yaml_path(&path)
}

mod common;
use common::mcp;
use common::{free_port, yaml_path};

/// A gateway whose one target strips `actor` and `role` and sets `actor` from
/// the token; the mock upstream echoes the arguments it received.
struct Harness {
    client: mcp::Client,
    shutdown: CancellationToken,
    _jwks: tempfile::TempDir,
}

impl Harness {
    /// With `bearer`, the route verifies it (`jwtAuth`); without, the route
    /// runs no auth at all, so calls arrive with no claims.
    async fn start(bearer: Option<&str>) -> Harness {
        let port = free_port();
        let (_, jwks) = keys();
        let dir = tempfile::tempdir().expect("should create a temp dir");
        let path = dir.path().join("jwks.json");
        std::fs::write(&path, jwks.to_string()).expect("should write the JWKS");
        let auth = match bearer {
            Some(_) => format!(
                "            policies:\n\
                 \x20             jwtAuth:\n\
                 \x20               issuer: {ISSUER}\n\
                 \x20               audiences: [\"{RESOURCE}\"]\n\
                 \x20               jwks:\n\
                 \x20                 file: \"{}\"\n",
                yaml_path(&path)
            ),
            None => String::new(),
        };
        let yaml = format!(
            r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: mcp
            matches:
              - path:
                  pathPrefix: /mcp
{auth}            backends:
              - mcp:
                  targets:
                    - name: board
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: board
                          MOCK_TOOLS: "comment"
                          MOCK_ECHO_ARGS: "1"
                      identity:
                        strip: [actor, role]
                        set:
                          actor: "lel-gateway/{{{{jwt.lel.lens}}}}"
"#,
            server = mock_server()
        );

        let config = Config::from_yaml(&yaml).expect("config should parse");
        config.validate().expect("config should validate");
        let gateway = common::build(&config, None)
            .await
            .expect("gateway should build");
        let shutdown = CancellationToken::new();
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
        let _serving = common::serve(gateway, vec![addr], shutdown.clone())
            .await
            .expect("gateway should bind");

        let mut config = mcp::HttpConfig::new(format!("http://127.0.0.1:{port}/mcp"));
        config.bearer_token = bearer.map(str::to_owned);
        let client = mcp::connect_with(config)
            .await
            .expect("client should complete the MCP handshake");
        Harness {
            client,
            shutdown,
            _jwks: dir,
        }
    }

    async fn call(&self, args: &str) -> Result<String, String> {
        let args = rusty_mcp_client_native::json::Value::from_json_str(args).expect("valid JSON");
        self.client
            .call_tool("board_comment", Some(args))
            .await
            .map(|r| mcp::text(&r.content))
            .map_err(|err| err.to_string())
    }

    async fn stop(self) {
        let _ = self.client.close().await;
        self.shutdown.cancel();
    }
}

#[rusty_tokio::test]
async fn the_upstream_sees_the_tokens_identity_not_the_callers() {
    let bearer = token(json!({ "sub": "p/c9/adversary/1", "lel.lens": "adversary" }));
    let harness = Harness::start(Some(&bearer)).await;

    let reply = harness
        .call(r#"{"id":"42","text":"hi","actor":"nano","role":"admin"}"#)
        .await
        .expect("the call should go through");
    let sent: Value = serde_json::from_str(
        reply
            .strip_prefix("board:comment ")
            .expect("the mock echoes its arguments"),
    )
    .expect("echoed arguments are JSON");
    assert_eq!(
        sent["actor"], "lel-gateway/adversary",
        "stamped from the token"
    );
    assert!(sent.get("role").is_none(), "stripped");
    assert_eq!(sent["text"], "hi", "everything else passes through");

    harness.stop().await;
}

#[rusty_tokio::test]
async fn a_call_without_a_token_is_refused_not_sent_unstamped() {
    let harness = Harness::start(None).await;
    let refused = harness.call(r#"{"text":"hi","actor":"nano"}"#).await;
    let message = refused.expect_err("no token, no identity, no call");
    assert!(message.contains("verified token"), "says why: {message}");
    harness.stop().await;
}

#[rusty_tokio::test]
async fn a_token_missing_the_claim_is_refused() {
    let bearer = token(json!({ "sub": "p/c9/adversary/1" }));
    let harness = Harness::start(Some(&bearer)).await;
    let refused = harness.call(r#"{"text":"hi"}"#).await;
    let message = refused.expect_err("the template needs lel.lens");
    assert!(message.contains("lel.lens"), "names the claim: {message}");
    harness.stop().await;
}
