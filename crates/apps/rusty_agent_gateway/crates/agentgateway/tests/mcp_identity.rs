//! End-to-end tests for a target's `identity` (Let Em Loose #1376, D223).
//!
//! The unit tests in `agentgateway-mcp` cover the templates. These cover the
//! wiring: claims come from a token this gateway verified, a forged actor in
//! the call is replaced before the upstream sees it, and a call with no usable
//! token is refused rather than sent anonymous.

use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use agentgateway_config::Config;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rusty_mcp_server::json::Value as Args;
use rusty_tokio::sync::CancellationToken;
use serde_json::{Value, json};

mod common;
use common::mcp;
use common::{free_port, yaml_path};

const ISSUER: &str = "https://auth.example.com";
const KID: &str = "identity-test-key";
const RESOURCE: &str = "https://gateway.example.com/mcp";

static KEY: std::sync::OnceLock<(Vec<u8>, Value)> = std::sync::OnceLock::new();

fn keys() -> (EncodingKey, Value) {
    let (der, jwks) = KEY.get_or_init(|| {
        use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};

        let private =
            RsaPrivateKey::new(&mut rand::thread_rng(), 2048).expect("should generate a key");
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

fn token(extra: Value) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after the epoch")
        .as_secs();
    let mut claims = json!({ "iss": ISSUER, "aud": RESOURCE, "exp": now + 3600 });
    if let (Some(claims), Value::Object(extra)) = (claims.as_object_mut(), extra) {
        claims.extend(extra);
    }
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.into());
    encode(&header, &claims, &keys().0).expect("should sign")
}

fn mock_server() -> String {
    let mut path = std::env::current_exe().expect("test binary should have a path");
    path.pop();
    path.pop();
    path.push("examples");
    path.push(format!("mock_mcp_server{}", std::env::consts::EXE_SUFFIX));
    assert!(path.exists(), "fixture not built at {}", path.display());
    yaml_path(&path)
}

/// The `board` target: stamps `actor` from the token.
const BOARD: &str = r#"                    - name: board
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: board
                          MOCK_TOOLS: "card_comment"
                          MOCK_ECHO_ARGS: "1"
                      identity:
                        strip: { "*": [actor, agent_key] }
                        set: { actor: "lel-gateway/{{jwt.lel.lens}}-{{jwt.lel.claim:4}}" }"#;

/// A `board` target whose session children are launched from the token: the
/// child's label is the token's `lel.claim`.
const LAUNCHED: &str = r#"                    - name: board
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: probe
                          MOCK_TOOLS: "card_comment"
                        session:
                          cmd: "{server}"
                          env:
                            MOCK_LABEL: "{{jwt.lel.claim}}"
                            MOCK_TOOLS: "card_comment""#;

struct Harness {
    client: mcp::Client,
    shutdown: CancellationToken,
    _jwks: tempfile::TempDir,
}

impl Harness {
    /// A gateway whose `board` target stamps `actor` from the token, with
    /// `jwtAuth` required, and a client presenting `bearer`.
    async fn start(bearer: &str) -> Harness {
        Self::start_with(bearer, BOARD).await
    }

    /// A gateway with one target, `target` (YAML under `stdio:`'s parent,
    /// with `{server}` for the mock's path), behind `jwtAuth`.
    async fn start_with(bearer: &str, target: &str) -> Harness {
        let port = free_port();
        let dir = tempfile::tempdir().expect("should create a temp dir");
        let jwks = dir.path().join("jwks.json");
        std::fs::write(&jwks, keys().1.to_string()).expect("should write the JWKS");

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
            policies:
              jwtAuth:
                issuer: {ISSUER}
                audiences: ["{RESOURCE}"]
                jwks:
                  file: "{jwks}"
            backends:
              - mcp:
                  targets:
{target}
"#,
            jwks = yaml_path(&jwks),
            target = target.replace("{server}", &mock_server()),
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
        config.bearer_token = Some(bearer.to_owned());
        let client = mcp::connect_with(config)
            .await
            .expect("client should complete the MCP handshake");
        Harness {
            client,
            shutdown,
            _jwks: dir,
        }
    }

    async fn comment(&self, args: &str) -> Result<String, String> {
        let args = Args::from_json_str(args).expect("test arguments are JSON");
        let result = self
            .client
            .call_tool("board_card_comment", Some(args))
            .await
            .map_err(|err| err.to_string())?;
        Ok(mcp::text(&result.content))
    }

    async fn stop(self) {
        let _ = self.client.close().await;
        self.shutdown.cancel();
    }
}

/// The arguments the mock saw, parsed from its echo.
fn seen(echo: &str) -> Value {
    let json = echo
        .strip_prefix("board:card_comment ")
        .expect("the mock echoes its label first");
    serde_json::from_str(json).expect("the mock echoes JSON")
}

#[rusty_tokio::test]
async fn the_upstream_sees_the_tokens_actor_not_the_forged_one() {
    let harness = Harness::start(&token(json!({
        "sub": "rusty_mill/c-77/builder-rust/1",
        "lel.lens": "builder-rust",
        "lel.claim": "c-77xyz",
    })))
    .await;

    let echo = harness
        .comment(r#"{"id":"42","text":"hi","actor":"nano","agent_key":"stolen"}"#)
        .await
        .expect("the call should succeed");
    let args = seen(&echo);
    assert_eq!(args["actor"], "lel-gateway/builder-rust-c-77");
    assert_eq!(args["text"], "hi");
    assert!(args.get("agent_key").is_none(), "got: {args}");

    harness.stop().await;
}

#[rusty_tokio::test]
async fn a_token_without_the_lens_claim_is_refused() {
    let harness = Harness::start(&token(json!({ "sub": "someone" }))).await;

    let err = harness
        .comment(r#"{"actor":"nano"}"#)
        .await
        .expect_err("a call that can't be stamped must not reach the upstream");
    assert!(err.contains("lel.lens"), "got: {err}");

    harness.stop().await;
}

#[rusty_tokio::test]
async fn a_session_child_is_launched_from_the_token() {
    let harness = Harness::start_with(
        &token(json!({ "sub": "rusty_mill/c-77/builder-rust/1", "lel.claim": "c-77" })),
        LAUNCHED,
    )
    .await;

    let echo = harness
        .comment("{}")
        .await
        .expect("the session's child should answer");
    assert!(echo.starts_with("c-77:card_comment"), "got: {echo}");

    harness.stop().await;
}

#[rusty_tokio::test]
async fn a_claim_that_could_leave_its_segment_launches_nothing() {
    let harness = Harness::start_with(
        &token(json!({ "sub": "someone", "lel.claim": ".." })),
        LAUNCHED,
    )
    .await;

    let err = harness
        .comment("{}")
        .await
        .expect_err("a launch with `..` must not start");
    assert!(err.contains("lel.claim"), "got: {err}");

    harness.stop().await;
}
