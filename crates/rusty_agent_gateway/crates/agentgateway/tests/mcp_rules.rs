//! End-to-end tests for `mcpAuthorization.rules`.
//!
//! The unit tests in `agentgateway-mcp` cover what the rules decide. These
//! cover the wiring those cannot reach: that a rule is consulted on a real
//! `tools/call` and not only on the listing, that `jwt.*` resolves to claims
//! from a token this gateway actually verified, and that the name a rule sees
//! is the tool's own rather than the federated one.

use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rmcp::{
    ServiceExt, model::CallToolRequestParams, service::RunningService,
    transport::StreamableHttpClientTransport,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const ISSUER: &str = "https://auth.example.com";
const KID: &str = "gateway-test-key";
const RESOURCE: &str = "https://gateway.example.com/mcp";

static PRIMARY: std::sync::OnceLock<(Vec<u8>, Value)> = std::sync::OnceLock::new();

fn keys() -> (EncodingKey, Value) {
    let (der, jwks) = PRIMARY.get_or_init(|| {
        use base64::Engine as _;
        use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};

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
use common::{free_port, yaml_path};

struct Harness {
    client: RunningService<rmcp::RoleClient, ()>,
    shutdown: CancellationToken,
    /// Holds the JWKS on disk for as long as the gateway is running.
    _jwks: Option<tempfile::TempDir>,
}

impl Harness {
    /// Boot a gateway whose one route carries `rules`, and connect a client.
    ///
    /// `rules` arrives as bare YAML lines and is indented here. When `bearer`
    /// is set the route also carries `jwtAuth`, so `jwt.*` has something to
    /// resolve against.
    async fn start(rules: &[&str], bearer: Option<&str>) -> Harness {
        let port = free_port().await;
        let rules = rules
            .iter()
            .map(|line| format!("                  {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        let (auth, dir) = match bearer {
            Some(_) => {
                let (_, jwks) = keys();
                let dir = tempfile::tempdir().expect("should create a temp dir");
                let path = dir.path().join("jwks.json");
                std::fs::write(&path, jwks.to_string()).expect("should write the JWKS");
                (
                    format!(
                        "              jwtAuth:\n\
                         \x20               issuer: {ISSUER}\n\
                         \x20               audiences: [\"{RESOURCE}\"]\n\
                         \x20               jwks:\n\
                         \x20                 file: \"{}\"\n",
                        yaml_path(&path)
                    ),
                    Some(dir),
                )
            }
            None => (String::new(), None),
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
            policies:
{auth}              mcpAuthorization:
                rules:
{rules}
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                          MOCK_TOOLS: "echo,ping,delete"
                    - name: beta
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: beta
                          MOCK_TOOLS: "echo,delete"
"#,
            server = mock_server()
        );

        let config = Config::from_yaml(&yaml).expect("config should parse");
        config.validate().expect("config should validate");
        let gateway = Gateway::build(&config, None)
            .await
            .expect("gateway should build");

        let shutdown = CancellationToken::new();
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
        let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
            .await
            .expect("gateway should bind");

        let uri = format!("http://127.0.0.1:{port}/mcp");
        let mut config =
            rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                uri,
            );
        if let Some(bearer) = bearer {
            // The builder adds the `Bearer ` prefix itself.
            config = config.auth_header(bearer);
        }
        let transport = StreamableHttpClientTransport::with_client(reqwest::Client::new(), config);

        let client = ().serve(transport).await.expect("client should complete the MCP handshake");

        Harness {
            client,
            shutdown,
            _jwks: dir,
        }
    }

    async fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .client
            .list_all_tools()
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
            .call_tool(CallToolRequestParams::new(name.to_string()))
            .await
            .map_err(|err| err.to_string())?;
        Ok(result
            .content
            .iter()
            .filter_map(|block| block.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join(""))
    }

    async fn stop(self) {
        let _ = self.client.cancel().await;
        self.shutdown.cancel();
    }
}

#[tokio::test]
async fn an_allow_rule_permits_only_what_it_names() {
    let harness = Harness::start(&[r#"- 'mcp.tool.name == "echo"'"#], None).await;

    assert_eq!(
        harness.tool_names().await,
        vec!["alpha_echo", "beta_echo"],
        "one allow rule makes the set an allow-list"
    );
    assert_eq!(harness.call("alpha_echo").await, Ok("alpha:echo".into()));

    harness.stop().await;
}

#[tokio::test]
async fn a_rule_is_enforced_on_the_call_not_only_the_listing() {
    // The failure this pins down: hiding a tool from the catalogue while
    // leaving it callable is strictly worse than not hiding it, because the
    // operator believes it is gone.
    let harness = Harness::start(&[r#"- 'mcp.tool.name == "echo"'"#], None).await;

    assert!(
        !harness.tool_names().await.contains(&"alpha_delete".into()),
        "the tool should be absent from the listing"
    );
    let err = harness
        .call("alpha_delete")
        .await
        .expect_err("a name the caller was never shown must still be refused");
    assert!(err.contains("not permitted"), "got: {err}");

    harness.stop().await;
}

#[tokio::test]
async fn a_rule_matches_the_unqualified_name() {
    // `alpha_echo` is what the client sees; `echo` is what the rule sees. A
    // rule written against the federated name would never fire, and the route
    // would quietly serve nothing.
    let federated = Harness::start(&[r#"- 'mcp.tool.name == "alpha_echo"'"#], None).await;
    assert!(
        federated.tool_names().await.is_empty(),
        "a rule written against the federated name matches nothing"
    );
    federated.stop().await;

    let unqualified = Harness::start(&[r#"- 'mcp.tool.name == "echo"'"#], None).await;
    assert_eq!(
        unqualified.tool_names().await,
        vec!["alpha_echo", "beta_echo"]
    );
    unqualified.stop().await;
}

#[tokio::test]
async fn a_rule_can_name_the_target() {
    let harness = Harness::start(&[r#"- 'mcp.tool.target == "alpha"'"#], None).await;

    assert_eq!(
        harness.tool_names().await,
        vec!["alpha_delete", "alpha_echo", "alpha_ping"],
        "beta's tools are not on the allow-list"
    );
    assert!(harness.call("beta_echo").await.is_err());

    harness.stop().await;
}

#[tokio::test]
async fn a_deny_rule_alone_permits_everything_else() {
    let harness = Harness::start(&[r#"- deny: 'mcp.tool.name == "delete"'"#], None).await;

    assert_eq!(
        harness.tool_names().await,
        vec!["alpha_echo", "alpha_ping", "beta_echo"],
        "a pure deny set is a deny-list, so what it does not name survives"
    );
    assert!(harness.call("alpha_delete").await.is_err());
    assert_eq!(harness.call("alpha_ping").await, Ok("alpha:ping".into()));

    harness.stop().await;
}

#[tokio::test]
async fn deny_beats_allow() {
    let harness = Harness::start(
        &["- 'true'", r#"- deny: 'mcp.tool.name == "delete"'"#],
        None,
    )
    .await;

    assert!(!harness.tool_names().await.contains(&"alpha_delete".into()));
    assert!(harness.call("alpha_delete").await.is_err());
    assert_eq!(harness.call("alpha_echo").await, Ok("alpha:echo".into()));

    harness.stop().await;
}

#[tokio::test]
async fn a_claim_from_the_verified_token_reaches_a_rule() {
    // The wiring this file exists for: `jwt.sub` resolving to a claim the
    // gateway itself verified, not to anything the caller set.
    let bearer = token(json!({"sub": "test-user"}));
    let harness = Harness::start(
        &[r#"- 'jwt.sub == "test-user" && mcp.tool.name == "echo"'"#],
        Some(&bearer),
    )
    .await;

    assert_eq!(harness.tool_names().await, vec!["alpha_echo", "beta_echo"]);
    assert_eq!(harness.call("alpha_echo").await, Ok("alpha:echo".into()));

    harness.stop().await;
}

#[tokio::test]
async fn a_rule_refuses_a_caller_whose_claim_does_not_match() {
    let bearer = token(json!({"sub": "someone-else"}));
    let harness = Harness::start(
        &[r#"- 'jwt.sub == "test-user" && mcp.tool.name == "echo"'"#],
        Some(&bearer),
    )
    .await;

    assert!(
        harness.tool_names().await.is_empty(),
        "a valid token for the wrong subject authorizes nothing"
    );
    assert!(harness.call("alpha_echo").await.is_err());

    harness.stop().await;
}

#[tokio::test]
async fn a_nested_claim_resolves() {
    let bearer = token(json!({"sub": "u", "nested": {"key": "value"}}));
    let harness = Harness::start(&[r#"- 'jwt.nested.key == "value"'"#], Some(&bearer)).await;

    assert!(!harness.tool_names().await.is_empty());
    assert_eq!(harness.call("alpha_echo").await, Ok("alpha:echo".into()));

    harness.stop().await;
}

#[tokio::test]
async fn a_require_refuses_when_there_is_no_token_to_read() {
    // No `jwtAuth` on this route, so `jwt` never resolves. A `require` that
    // cannot be evaluated refuses -- which is the reason to prefer it to
    // `deny`, where the same expression would permit.
    let harness = Harness::start(&[r#"- require: 'jwt.sub == "test-user"'"#], None).await;

    assert!(harness.tool_names().await.is_empty());
    assert!(harness.call("alpha_echo").await.is_err());

    harness.stop().await;
}

#[tokio::test]
async fn a_rule_that_does_not_compile_stops_the_gateway_booting() {
    // Rather than skipping the rule, which would turn a typo in an allow rule
    // into a route that serves nothing and a typo in a deny rule into one that
    // refuses nothing.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - policies:
              mcpAuthorization:
                rules:
                  - 'mcp.tool.name =='
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
"#,
        server = mock_server()
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    let err = Gateway::build(&config, None)
        .await
        .err()
        .expect("a rule that does not compile should be a startup failure");
    let err = err.to_string();
    assert!(err.contains("rules[0]"), "got: {err}");
}

#[tokio::test]
async fn a_rule_naming_two_modes_is_rejected() {
    // `{allow: ..., deny: ...}` has no single meaning, and picking one would
    // quietly enforce something nobody wrote.
    let yaml = r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              mcpAuthorization:
                rules:
                  - allow: 'true'
                    deny: 'true'
            backends:
              - host: "127.0.0.1:9"
"#;
    let err = Config::from_yaml(yaml).expect_err("should not parse");
    assert!(err.to_string().contains("only one of"), "got: {err}");
}
