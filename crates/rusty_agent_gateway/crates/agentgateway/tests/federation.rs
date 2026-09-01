//! End-to-end tests: a real MCP client, over a real socket, through the
//! gateway, into real subprocess MCP servers.
//!
//! The targets are `examples/mock_mcp_server.rs`, which echoes its own label
//! in every tool result. That is the point — it lets these tests assert a call
//! reached the *right* target, not merely that it reached one.

use std::net::SocketAddr;

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use rmcp::{
    ServiceExt, model::CallToolRequestParams, service::RunningService,
    transport::StreamableHttpClientTransport,
};
use tokio_util::sync::CancellationToken;

/// Path to the compiled fixture server.
///
/// Cargo puts examples in `target/<profile>/examples`, and the test binary
/// itself in `target/<profile>/deps`, so this walks across rather than
/// assuming a target directory location.
fn mock_server() -> String {
    let mut path = std::env::current_exe().expect("test binary should have a path");
    path.pop(); // deps/
    path.pop(); // <profile>/
    path.push("examples");
    // Windows names the built example `mock_mcp_server.exe`; `EXE_SUFFIX` is
    // empty everywhere else.
    path.push(format!("mock_mcp_server{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.exists(),
        "fixture not built at {}; `cargo test` builds examples, so this means \
         the example failed to compile",
        path.display()
    );
    path.display().to_string()
}

mod common;
use common::free_port;

struct Harness {
    client: RunningService<rmcp::RoleClient, ()>,
    shutdown: CancellationToken,
}

impl Harness {
    /// Boot a gateway from `config_template`, with `{server}` replaced by the
    /// fixture path and `{port}` by a free port, then connect a client to it.
    async fn start(config_template: &str) -> Harness {
        let port = free_port().await;
        let yaml = config_template
            .replace("{server}", &mock_server())
            .replace("{port}", &port.to_string());

        let config = Config::from_yaml(&yaml).expect("config should parse");
        config.validate().expect("config should validate");

        let gateway = Gateway::build(&config, None)
            .await
            .expect("gateway should build and reach its targets");

        let shutdown = CancellationToken::new();
        let addr: SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .expect("address should parse");
        // The returned future only joins the accept loops; these tests drive
        // the gateway over MCP and cancel it directly.
        let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
            .await
            .expect("gateway should bind");

        let transport =
            StreamableHttpClientTransport::from_uri(format!("http://127.0.0.1:{port}/mcp"));
        let client = ()
            .serve(transport)
            .await
            .expect("client should complete the MCP handshake through the gateway");

        Harness { client, shutdown }
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

        let text = result
            .content
            .iter()
            .filter_map(|block| block.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("");
        Ok(text)
    }

    async fn stop(self) {
        let _ = self.client.cancel().await;
        self.shutdown.cancel();
    }
}

const TWO_TARGETS: &str = r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - matches:
              - path:
                  pathPrefix: /mcp
            policies:
              cors:
                allowOrigins: ["*"]
                exposeHeaders: ["Mcp-Session-Id"]
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                    - name: beta
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: beta
"#;

#[tokio::test]
async fn two_targets_federate_into_one_endpoint() {
    let harness = Harness::start(TWO_TARGETS).await;

    assert_eq!(
        harness.tool_names().await,
        vec!["alpha_echo", "alpha_ping", "beta_echo", "beta_ping"],
        "both targets' tools appear once, qualified by target"
    );

    harness.stop().await;
}

#[tokio::test]
async fn a_call_reaches_the_target_that_owns_the_tool() {
    let harness = Harness::start(TWO_TARGETS).await;

    // Both targets export `echo`. Without prefixing these are the same name,
    // so this is the assertion that federation actually routes rather than
    // picking whichever target sorted first.
    assert_eq!(
        harness.call("alpha_echo").await,
        Ok("alpha:echo".to_string())
    );
    assert_eq!(harness.call("beta_echo").await, Ok("beta:echo".to_string()));

    harness.stop().await;
}

#[tokio::test]
async fn an_unadvertised_tool_name_is_rejected() {
    let harness = Harness::start(TWO_TARGETS).await;

    let err = harness
        .call("gamma_echo")
        .await
        .expect_err("a name from no known target must not route anywhere");
    assert!(
        err.contains("gamma_echo"),
        "error should name the tool: {err}"
    );

    harness.stop().await;
}

const FILTERED: &str = r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - matches:
              - path:
                  pathPrefix: /mcp
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                      filters:
                        - action: deny
                          matcher: "^ping$"
"#;

#[tokio::test]
async fn a_filtered_tool_is_hidden_and_uncallable() {
    let harness = Harness::start(FILTERED).await;

    assert_eq!(
        harness.tool_names().await,
        vec!["alpha_echo"],
        "the denied tool is absent from the catalogue"
    );

    // The real test: a client that already knows the name cannot call it.
    // Filtering only the listing would leave this working.
    let err = harness
        .call("alpha_ping")
        .await
        .expect_err("a filtered tool must not be callable");
    assert!(
        err.contains("alpha_ping"),
        "error should name the tool: {err}"
    );

    harness.stop().await;
}

const AUTHORIZED: &str = r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - matches:
              - path:
                  pathPrefix: /mcp
            policies:
              mcpAuthorization:
                denyTools: ["^beta_"]
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                    - name: beta
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: beta
"#;

#[tokio::test]
async fn route_authorization_bans_one_target_but_not_the_other() {
    let harness = Harness::start(AUTHORIZED).await;

    assert_eq!(
        harness.tool_names().await,
        vec!["alpha_echo", "alpha_ping"],
        "beta's tools are denied by route policy"
    );

    assert_eq!(
        harness.call("alpha_echo").await,
        Ok("alpha:echo".to_string()),
        "the same tool name on a permitted target still works"
    );

    let err = harness
        .call("beta_echo")
        .await
        .expect_err("a denied tool must not be callable");
    assert!(
        err.contains("beta_echo"),
        "error should name the tool: {err}"
    );

    harness.stop().await;
}

#[tokio::test]
async fn a_route_that_does_not_match_is_a_404() {
    let port = free_port().await;
    let yaml = TWO_TARGETS
        .replace("{server}", &mock_server())
        .replace("{port}", &port.to_string());
    let config = Config::from_yaml(&yaml).expect("config should parse");
    let gateway = Gateway::build(&config, None)
        .await
        .expect("gateway should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    // The returned future only joins the accept loops; the tests drive the
    // gateway over HTTP and cancel it directly, so it is deliberately dropped.
    let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    let response = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/not-the-mcp-route"))
        .send()
        .await
        .expect("request should reach the gateway");
    assert_eq!(response.status(), 404);

    shutdown.cancel();
}
