//! End-to-end tests for `lelPolicy` (Let Em Loose #1378, D223).
//!
//! The unit tests in `agentgateway-mcp` cover parsing and selection. These
//! cover the wiring: policies load from a directory, a verified token picks
//! one by `lel.lens` and `lel.policy`, the listing is filtered, a hidden tool
//! can't be called, and an unknown lens sees nothing.

use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use agentgateway_config::Config;
use agentgateway_mcp::policy_digest;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rusty_tokio::sync::CancellationToken;
use serde_json::{Value, json};

mod common;
use common::mcp;
use common::{free_port, yaml_path};

const ISSUER: &str = "https://auth.example.com";
const KID: &str = "lens-test-key";
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

const CRITIC: &str = "schema = 1\nlens = \"critic\"\nrole = \"critic\"\n\n[tools]\nallow = [\"sandbox_read\", \"sandbox_list\"]\n";

const BUILDER: &str = "schema = 1\nlens = \"builder\"\nrole = \"builder\"\n\n[tools]\nallow = [\"sandbox_*\"]\ndeny = [\"sandbox_rm\"]\n";

struct Harness {
    client: mcp::Client,
    shutdown: CancellationToken,
    _dir: tempfile::TempDir,
}

impl Harness {
    /// A gateway with `jwtAuth` and `lelPolicy` over a `sandbox` target, and a
    /// client presenting `bearer`.
    async fn start(bearer: &str) -> Harness {
        let port = free_port();
        let dir = tempfile::tempdir().expect("should create a temp dir");
        let jwks = dir.path().join("jwks.json");
        std::fs::write(&jwks, keys().1.to_string()).expect("should write the JWKS");
        let policies = dir.path().join("lenses");
        std::fs::create_dir(&policies).expect("should create the policy dir");
        std::fs::write(policies.join("critic.toml"), CRITIC).expect("should write");
        std::fs::write(policies.join("builder.toml"), BUILDER).expect("should write");
        std::fs::write(policies.join("README.md"), "not a policy").expect("should write");

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
              lelPolicy:
                dir: "{policies}"
            backends:
              - mcp:
                  targets:
                    - name: sandbox
                      mode: pooled
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: sandbox
                          MOCK_TOOLS: "read,list,exec,rm"
"#,
            jwks = yaml_path(&jwks),
            policies = yaml_path(&policies),
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
        config.bearer_token = Some(bearer.to_owned());
        let client = mcp::connect_with(config)
            .await
            .expect("client should complete the MCP handshake");
        Harness {
            client,
            shutdown,
            _dir: dir,
        }
    }

    async fn tools(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .client
            .list_tools()
            .await
            .expect("tools/list should succeed")
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        names.sort();
        names
    }

    async fn call(&self, name: &str) -> Result<String, String> {
        let result = self
            .client
            .call_tool(name, None)
            .await
            .map_err(|err| err.to_string())?;
        Ok(mcp::text(&result.content))
    }

    async fn stop(self) {
        let _ = self.client.close().await;
        self.shutdown.cancel();
    }
}

fn lens(name: &str, file: &str) -> String {
    token(json!({ "lel.lens": name, "lel.policy": policy_digest(file.as_bytes()) }))
}

#[rusty_tokio::test]
async fn a_critic_token_sees_and_calls_only_read_tools() {
    let harness = Harness::start(&lens("critic", CRITIC)).await;
    assert_eq!(harness.tools().await, ["sandbox_list", "sandbox_read"]);
    assert_eq!(
        harness.call("sandbox_read").await,
        Ok("sandbox:read".into())
    );
    let err = harness
        .call("sandbox_exec")
        .await
        .expect_err("a tool hidden from the listing must be refused on call");
    assert!(err.contains("not permitted"), "got: {err}");
    harness.stop().await;
}

#[rusty_tokio::test]
async fn a_builder_token_gets_its_own_policy() {
    let harness = Harness::start(&lens("builder", BUILDER)).await;
    assert_eq!(
        harness.tools().await,
        ["sandbox_exec", "sandbox_list", "sandbox_read"],
        "deny wins over the glob"
    );
    harness.stop().await;
}

#[rusty_tokio::test]
async fn an_unknown_lens_or_a_stale_digest_sees_nothing() {
    for bearer in [lens("admin", CRITIC), lens("critic", BUILDER)] {
        let harness = Harness::start(&bearer).await;
        assert!(harness.tools().await.is_empty());
        assert!(harness.call("sandbox_read").await.is_err());
        harness.stop().await;
    }
}
