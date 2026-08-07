//! End-to-end tests for federated prompts and resources, and the
//! `mcpAuthorization.rules` that gate them.
//!
//! A real MCP client, over a real socket, into real subprocess MCP servers.
//! The point of doing it at this level is that prompts and resources are
//! federated differently from each other — prompts take the `target_name`
//! prefix, resources take a widened URI scheme — and both have to round-trip
//! back to the right upstream.

use std::net::SocketAddr;

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use rmcp::{
    ServiceExt,
    model::{GetPromptRequestParams, ReadResourceRequestParams},
    service::RunningService,
    transport::StreamableHttpClientTransport,
};
use tokio_util::sync::CancellationToken;

async fn free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind");
    listener.local_addr().expect("should have an addr").port()
}

fn mock_server() -> String {
    let mut path = std::env::current_exe().expect("test binary should have a path");
    path.pop(); // deps/
    path.pop(); // <profile>/
    path.push("examples");
    path.push("mock_mcp_server");
    assert!(path.exists(), "fixture not built at {}", path.display());
    path.display().to_string()
}

struct Harness {
    client: RunningService<rmcp::RoleClient, ()>,
    shutdown: CancellationToken,
}

impl Harness {
    /// Boot a gateway over the given YAML, substituting `{server}` and `{port}`.
    async fn start(body: &str) -> Harness {
        let port = free_port().await;
        let yaml = format!(
            r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - matches:
              - path:
                  pathPrefix: /mcp
{body}
"#
        )
        .replace("{server}", &mock_server());

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

        let client = ()
            .serve(StreamableHttpClientTransport::from_uri(format!(
                "http://127.0.0.1:{port}/mcp"
            )))
            .await
            .expect("client should complete the MCP handshake");

        Harness { client, shutdown }
    }

    async fn prompt_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .client
            .list_all_prompts()
            .await
            .expect("prompts/list should work")
            .into_iter()
            .map(|p| p.name)
            .collect();
        names.sort();
        names
    }

    async fn get_prompt(&self, name: &str) -> Result<String, String> {
        let result = self
            .client
            .get_prompt(GetPromptRequestParams::new(name))
            .await
            .map_err(|err| err.to_string())?;
        Ok(result
            .messages
            .iter()
            .filter_map(|m| m.content.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join(""))
    }

    async fn resource_uris(&self) -> Vec<String> {
        let mut uris: Vec<String> = self
            .client
            .list_all_resources()
            .await
            .expect("resources/list should work")
            .into_iter()
            .map(|r| r.uri)
            .collect();
        uris.sort();
        uris
    }

    async fn read_resource(&self, uri: &str) -> Result<(String, String), String> {
        let result = self
            .client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .map_err(|err| err.to_string())?;
        let content = result.contents.first().expect("one content block");
        match content {
            rmcp::model::ResourceContents::TextResourceContents { uri, text, .. } => {
                Ok((uri.clone(), text.clone()))
            }
            other => panic!("expected text contents, got {other:?}"),
        }
    }

    async fn stop(self) {
        let _ = self.client.cancel().await;
        self.shutdown.cancel();
    }
}

/// Two targets, both with prompts and resources.
const TWO_TARGETS: &str = r#"            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                          MOCK_TOOLS: "echo"
                          MOCK_PROMPTS: "summarize,leak"
                          MOCK_RESOURCES: "memo:insights,file:///secret"
                    - name: beta
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: beta
                          MOCK_TOOLS: "echo"
                          MOCK_PROMPTS: "summarize"
                          MOCK_RESOURCES: "memo:notes""#;

#[tokio::test]
async fn prompts_are_federated_under_a_qualified_name() {
    let harness = Harness::start(TWO_TARGETS).await;

    assert_eq!(
        harness.prompt_names().await,
        vec!["alpha_leak", "alpha_summarize", "beta_summarize"],
        "prompts take the same target prefix as tools, so two targets can \
         both export `summarize`"
    );

    // And each routes back to the target it came from.
    assert_eq!(
        harness.get_prompt("alpha_summarize").await,
        Ok("alpha:summarize".into())
    );
    assert_eq!(
        harness.get_prompt("beta_summarize").await,
        Ok("beta:summarize".into())
    );

    harness.stop().await;
}

#[tokio::test]
async fn resources_are_federated_by_widening_the_uri_scheme() {
    let harness = Harness::start(TWO_TARGETS).await;

    assert_eq!(
        harness.resource_uris().await,
        vec![
            "alpha+file:///secret",
            "alpha+memo:insights",
            "beta+memo:notes"
        ],
        "a resource is identified by URI, so it cannot take the `target_name` \
         treatment; the scheme is widened instead"
    );

    harness.stop().await;
}

#[tokio::test]
async fn a_resource_read_round_trips_through_the_gateway() {
    let harness = Harness::start(TWO_TARGETS).await;

    let (uri, text) = harness
        .read_resource("alpha+memo:insights")
        .await
        .expect("the read should succeed");

    assert_eq!(
        text, "alpha:memo:insights",
        "the upstream should have been asked for its own URI, not the federated one"
    );
    assert_eq!(
        uri, "alpha+memo:insights",
        "and the contents should come back re-qualified, or no client could \
         read the URI back to us"
    );

    harness.stop().await;
}

#[tokio::test]
async fn a_prompt_rule_gates_prompts_on_the_listing_and_the_fetch() {
    let harness = Harness::start(&format!(
        r#"            policies:
              mcpAuthorization:
                rules:
                  - 'mcp.prompt.name == "summarize"'
                  - 'mcp.tool.name == "echo"'
                  - 'mcp.resource.name != ""'
{TWO_TARGETS}"#
    ))
    .await;

    assert_eq!(
        harness.prompt_names().await,
        vec!["alpha_summarize", "beta_summarize"],
        "`leak` should be filtered out of the listing"
    );

    // The gate runs on the fetch too. Filtering the listing alone is theatre:
    // nothing stops a client asking for a name it was never shown.
    let err = harness
        .get_prompt("alpha_leak")
        .await
        .expect_err("a prompt off the allow-list should not be fetchable");
    assert!(
        err.contains("prompt `alpha_leak` is not permitted"),
        "got: {err}"
    );

    harness.stop().await;
}

#[tokio::test]
async fn a_resource_rule_matches_on_the_targets_own_uri() {
    let harness = Harness::start(&format!(
        r#"            policies:
              mcpAuthorization:
                rules:
                  - 'mcp.resource.name.startsWith("memo:")'
                  - 'mcp.tool.name == "echo"'
                  - 'mcp.prompt.name != ""'
{TWO_TARGETS}"#
    ))
    .await;

    assert_eq!(
        harness.resource_uris().await,
        vec!["alpha+memo:insights", "beta+memo:notes"],
        "the rule sees `memo:insights`, not `alpha+memo:insights` -- so a rule \
         can name a resource without knowing what the gateway will prefix it with"
    );

    let err = harness
        .read_resource("alpha+file:///secret")
        .await
        .expect_err("a resource off the allow-list should not be readable");
    assert!(
        err.contains("resource `alpha+file:///secret` is not permitted"),
        "got: {err}"
    );

    harness.stop().await;
}

#[tokio::test]
async fn a_tool_allow_list_refuses_prompts_and_resources() {
    // The behaviour most likely to surprise, so it is pinned end to end.
    // Exactly one of `mcp.tool`, `mcp.prompt` and `mcp.resource` is bound per
    // call, so on a prompt the expression `mcp.tool.name == "echo"` does not
    // resolve and reads as false. With no other allow rule to satisfy, the
    // allow-list refuses every prompt and resource.
    let harness = Harness::start(&format!(
        r#"            policies:
              mcpAuthorization:
                rules:
                  - 'mcp.tool.name == "echo"'
{TWO_TARGETS}"#
    ))
    .await;

    assert!(
        harness.prompt_names().await.is_empty(),
        "a tool allow-list leaves nothing in the prompt space that can satisfy it"
    );
    assert!(harness.resource_uris().await.is_empty());

    // Tools still work, which is what makes this a trap rather than an outage.
    let tools: Vec<String> = harness
        .client
        .list_all_tools()
        .await
        .expect("tools/list should work")
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(tools.len(), 2, "{tools:?}");

    harness.stop().await;
}

#[tokio::test]
async fn a_deny_list_leaves_prompts_and_resources_alone() {
    // The mirror image: a deny-list names what is refused, so what it does not
    // name survives regardless of kind.
    let harness = Harness::start(&format!(
        r#"            policies:
              mcpAuthorization:
                rules:
                  - deny: 'mcp.prompt.name == "leak"'
{TWO_TARGETS}"#
    ))
    .await;

    assert_eq!(
        harness.prompt_names().await,
        vec!["alpha_summarize", "beta_summarize"]
    );
    assert_eq!(
        harness.resource_uris().await.len(),
        3,
        "resources are untouched by a prompt deny rule"
    );

    harness.stop().await;
}

#[tokio::test]
async fn a_rule_can_gate_on_the_target_as_well_as_the_name() {
    let harness = Harness::start(&format!(
        r#"            policies:
              mcpAuthorization:
                rules:
                  - 'mcp.prompt.target == "alpha"'
                  - 'mcp.tool.name != ""'
                  - 'mcp.resource.name != ""'
{TWO_TARGETS}"#
    ))
    .await;

    assert_eq!(
        harness.prompt_names().await,
        vec!["alpha_leak", "alpha_summarize"],
        "beta's prompts should be gone even though one shares a name with alpha's"
    );

    harness.stop().await;
}

#[tokio::test]
async fn resource_templates_are_gated_on_their_template() {
    let harness = Harness::start(&format!(
        r#"            policies:
              mcpAuthorization:
                rules:
                  - 'mcp.resource.name.startsWith("alpha:")'
{TWO_TARGETS}"#
    ))
    .await;

    let templates: Vec<String> = harness
        .client
        .list_all_resource_templates()
        .await
        .expect("resources/templates/list should work")
        .into_iter()
        .map(|t| t.uri_template)
        .collect();

    assert_eq!(
        templates,
        vec!["alpha+alpha:{id}"],
        "beta's template should be filtered out; alpha's is qualified like any \
         other resource URI"
    );

    harness.stop().await;
}

#[tokio::test]
async fn capabilities_follow_what_the_targets_actually_have() {
    // Claiming prompts the federation cannot serve would have clients calling
    // `prompts/list` only to be told the method does not exist.
    let harness = Harness::start(
        r#"            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                          MOCK_TOOLS: "echo""#,
    )
    .await;

    let info = harness
        .client
        .peer_info()
        .expect("the handshake should have carried server info");
    assert!(info.capabilities.tools.is_some());
    assert!(
        info.capabilities.prompts.is_none(),
        "no target has prompts, so the gateway must not advertise them"
    );
    assert!(info.capabilities.resources.is_none());

    harness.stop().await;
}

#[tokio::test]
async fn one_target_with_prompts_is_enough_to_advertise_them() {
    let harness = Harness::start(TWO_TARGETS).await;

    let info = harness
        .client
        .peer_info()
        .expect("the handshake should have carried server info");
    assert!(info.capabilities.prompts.is_some());
    assert!(info.capabilities.resources.is_some());

    harness.stop().await;
}

#[tokio::test]
async fn a_target_without_prompts_is_skipped_rather_than_reported_as_faulty() {
    // A missing capability is not a fault, and a mixed federation is the
    // normal case rather than the exception.
    let harness = Harness::start(
        r#"            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                          MOCK_TOOLS: "echo"
                          MOCK_PROMPTS: "summarize"
                    - name: beta
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: beta
                          MOCK_TOOLS: "echo""#,
    )
    .await;

    assert_eq!(harness.prompt_names().await, vec!["alpha_summarize"]);

    harness.stop().await;
}

#[tokio::test]
async fn an_unknown_prompt_or_resource_is_a_clean_error() {
    let harness = Harness::start(TWO_TARGETS).await;

    let err = harness
        .get_prompt("nosuch_prompt")
        .await
        .expect_err("an unrouteable prompt should not succeed");
    assert!(err.contains("unknown prompt"), "got: {err}");

    let err = harness
        .read_resource("nosuch+memo:x")
        .await
        .expect_err("an unrouteable resource should not succeed");
    assert!(err.contains("unknown resource"), "got: {err}");

    harness.stop().await;
}
