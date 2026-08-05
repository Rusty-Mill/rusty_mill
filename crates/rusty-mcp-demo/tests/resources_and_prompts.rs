//! End-to-end tests for resources and prompts against `DemoServer`.

use rmcp::{
    ClientHandler, ServiceExt,
    model::{
        ClientInfo, GetPromptRequestParams, ProtocolVersion, ReadResourceRequestParams,
        ResourceContents,
    },
    service::RunningService,
};

#[path = "../src/prompts.rs"]
mod prompts;
#[path = "../src/resources.rs"]
mod resources;
#[path = "../src/server.rs"]
mod server;
#[path = "../src/tools/mod.rs"]
mod tools;

use server::DemoServer;

/// A client pinned to 2026-07-28, so cache hints are in play.
#[derive(Clone)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info
    }
}

async fn connect() -> RunningService<rmcp::RoleClient, Client> {
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    tokio::spawn(async move {
        let running = DemoServer::new()
            .serve(server_transport)
            .await
            .expect("server starts");
        let _ = running.waiting().await;
    });

    Client
        .serve(client_transport)
        .await
        .expect("client connects")
}

fn text_of(contents: &ResourceContents) -> &str {
    match contents {
        ResourceContents::TextResourceContents { text, .. } => text,
        other => panic!("expected text contents, got {other:?}"),
    }
}

#[tokio::test]
async fn advertises_resource_and_prompt_capabilities() {
    let client = connect().await;
    let info = client.peer_info().expect("server info");

    assert!(
        info.capabilities.resources.is_some(),
        "resources capability"
    );
    assert!(info.capabilities.prompts.is_some(), "prompts capability");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn lists_resources_with_cache_hints() {
    let client = connect().await;

    let list = client.list_resources(None).await.expect("resources/list");
    let mut names: Vec<_> = list.resources.iter().map(|r| r.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["demo-config", "uptime"].map(String::from).to_vec());

    // SEP-2549 requires cache hints on list results under 2026-07-28.
    assert!(list.ttl_ms.is_some(), "missing ttlMs");
    assert!(list.cache_scope.is_some(), "missing cacheScope");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn lists_resource_templates() {
    let client = connect().await;

    let list = client
        .list_resource_templates(None)
        .await
        .expect("resources/templates/list");

    assert_eq!(list.resource_templates.len(), 1);
    assert_eq!(
        list.resource_templates[0].uri_template,
        "db://tables/{table}"
    );
    assert!(list.ttl_ms.is_some());

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn reads_a_static_resource() {
    let client = connect().await;

    let result = client
        .read_resource(ReadResourceRequestParams::new("config://demo"))
        .await
        .expect("resources/read");

    let text = text_of(&result.contents[0]);
    assert!(text.contains("\"greeting\""), "unexpected body: {text}");
    assert!(
        result.ttl_ms.is_some(),
        "read results carry cache hints too"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn reads_a_generated_resource() {
    let client = connect().await;

    let result = client
        .read_resource(ReadResourceRequestParams::new("status://uptime"))
        .await
        .expect("resources/read");

    // Produced per read rather than fixed at registration.
    assert!(text_of(&result.contents[0]).ends_with(" seconds"));

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn reads_through_a_uri_template() {
    let client = connect().await;

    let result = client
        .read_resource(ReadResourceRequestParams::new("db://tables/users"))
        .await
        .expect("resources/read");

    let body: serde_json::Value =
        serde_json::from_str(text_of(&result.contents[0])).expect("json body");
    assert_eq!(body["table"], "users");
    assert_eq!(body["columns"][0], "id");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn an_unknown_resource_is_an_error() {
    let client = connect().await;

    assert!(
        client
            .read_resource(ReadResourceRequestParams::new("config://nope"))
            .await
            .is_err(),
        "an unregistered URI should be rejected"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn a_template_variable_cannot_traverse_out_of_its_namespace() {
    let client = connect().await;

    // `{table}` never matches across `/`, so this falls through to not-found
    // rather than reaching the reader with a traversal payload.
    assert!(
        client
            .read_resource(ReadResourceRequestParams::new(
                "db://tables/../../etc/passwd"
            ))
            .await
            .is_err(),
        "a traversal attempt must not match the template"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn an_unknown_table_reports_bad_parameters() {
    let client = connect().await;

    // Matches the template, but the reader rejects it — distinct from a URI
    // that matches nothing at all.
    let err = client
        .read_resource(ReadResourceRequestParams::new("db://tables/nonexistent"))
        .await
        .expect_err("unknown table");

    assert!(
        err.to_string().contains("no such table"),
        "unexpected error: {err}"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn lists_prompts_with_descriptions() {
    let client = connect().await;

    let list = client.list_prompts(None).await.expect("prompts/list");
    let mut names: Vec<_> = list.prompts.iter().map(|p| p.name.clone()).collect();
    names.sort();
    assert_eq!(
        names,
        ["explain-error", "summarize"].map(String::from).to_vec()
    );

    for prompt in &list.prompts {
        assert!(
            prompt.description.as_ref().is_some_and(|d| !d.is_empty()),
            "prompt {} needs a description",
            prompt.name
        );
    }
    assert!(list.ttl_ms.is_some(), "prompts/list carries cache hints");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn gets_a_prompt_with_arguments() {
    let client = connect().await;

    let result = client
        .get_prompt(
            GetPromptRequestParams::new("summarize").with_arguments(
                serde_json::json!({ "text": "war and peace", "sentences": 2 })
                    .as_object()
                    .cloned()
                    .expect("object"),
            ),
        )
        .await
        .expect("prompts/get");

    assert_eq!(result.messages.len(), 1);
    let rendered = format!("{:?}", result.messages[0]);
    assert!(rendered.contains("war and peace"), "{rendered}");
    assert!(rendered.contains("about 2 sentences"), "{rendered}");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn an_optional_prompt_argument_may_be_omitted() {
    let client = connect().await;

    let result = client
        .get_prompt(
            GetPromptRequestParams::new("explain-error").with_arguments(
                serde_json::json!({ "error": "segfault" })
                    .as_object()
                    .cloned()
                    .expect("object"),
            ),
        )
        .await
        .expect("prompts/get without the optional argument");

    let rendered = format!("{:?}", result.messages[0]);
    assert!(rendered.contains("segfault"), "{rendered}");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn tools_still_work_alongside_resources_and_prompts() {
    // The handler now carries a tool router, a prompt router, a resource
    // registry and task support at once; this checks nothing shadowed anything.
    let client = connect().await;

    let tools = client.list_tools(None).await.expect("tools/list");
    assert_eq!(tools.tools.len(), 5);

    client.cancel().await.expect("cancel");
}
