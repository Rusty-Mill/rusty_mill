//! Completion and list pagination, over a real socket.
//!
//! The unit tests cover the registries directly. What only shows up end to end
//! is the part that matters most here: **a fresh handler is built per request
//! under Streamable HTTP**, so page two is served by a different handler
//! instance than page one. If any paging state lived in the handler rather than
//! in the cursor, a duplex or stdio test would never notice — one handler
//! serves that whole connection — and the failure would appear only in
//! production behind a load balancer.
//!
//! That is exactly the shape of the subscriptions bug this repo already hit
//! once, so pagination gets the same treatment.

use std::{net::SocketAddr, time::Duration};

use rmcp::{
    ClientLifecycleMode, ClientServiceExt, ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{
        ClientInfo, PaginatedRequestParams, ProtocolVersion, Reference, Resource, ResourceTemplate,
        ServerCapabilities, ServerInfo,
    },
    tool, tool_router,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use rusty_mcp::{
    HttpConfig, ServerConfig, Transport, completion::CompletionRegistry,
    resources::ResourceRegistry,
};

/// How many resources the test server registers.
const RESOURCE_COUNT: usize = 25;
/// How many it serves per page.
const PAGE_SIZE: usize = 10;
/// Tools per page. Six tools at two a page is three pages.
const TOOL_PAGE_SIZE: usize = 2;

/// A server whose registries are rebuilt from scratch for every request.
///
/// Deliberate: nothing is shared between requests, so anything that survives
/// from one page to the next has to have travelled through the cursor.
#[derive(Clone)]
struct PagedServer {
    resources: ResourceRegistry,
    completions: CompletionRegistry,
    tool_router: ToolRouter<Self>,
}

/// Six tools, named so that registration order and name order disagree.
#[tool_router(router = tool_router)]
impl PagedServer {
    #[tool(description = "f")]
    async fn zulu(&self) -> String {
        "z".into()
    }
    #[tool(description = "e")]
    async fn yankee(&self) -> String {
        "y".into()
    }
    #[tool(description = "d")]
    async fn xray(&self) -> String {
        "x".into()
    }
    #[tool(description = "c")]
    async fn charlie(&self) -> String {
        "c".into()
    }
    #[tool(description = "b")]
    async fn bravo(&self) -> String {
        "b".into()
    }
    #[tool(description = "a")]
    async fn alpha(&self) -> String {
        "a".into()
    }
}

impl PagedServer {
    fn new() -> Self {
        // Registered in reverse so registration order and URI order disagree.
        let mut resources = ResourceRegistry::new().with_page_size(PAGE_SIZE);
        for i in (0..RESOURCE_COUNT).rev() {
            resources =
                resources.with_text(Resource::new(format!("mem://r{i:03}"), "entry"), "content");
        }
        let resources = resources.with_template(
            ResourceTemplate::new("db://tables/{table}", "table"),
            |_req| async move { Ok(vec![]) },
        );

        let completions = CompletionRegistry::new()
            .with_values(
                Reference::for_prompt("explain-error"),
                "language",
                ["Rust", "python", "ruby"],
            )
            .with_completer(
                Reference::for_resource("db://tables/{table}"),
                "table",
                |_req| async move { Ok(vec!["users".to_string(), "orders".to_string()]) },
            );

        Self {
            resources,
            completions,
            tool_router: Self::tool_router(),
        }
    }
}

impl ServerHandler for PagedServer {
    fn get_info(&self) -> ServerInfo {
        rusty_mcp::server_info(
            "paged-server",
            "0.1.0",
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .enable_completions()
                .build(),
        )
    }

    rusty_mcp::forward_resource_methods!(resources);
    rusty_mcp::forward_completion_methods!(completions);
    rusty_mcp::forward_tool_methods!(tool_router, TOOL_PAGE_SIZE);
}

async fn free_port() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    listener.local_addr().expect("local addr")
}

async fn spawn_server() -> String {
    let addr = free_port().await;
    let config = ServerConfig {
        transport: Transport::Http(HttpConfig {
            bind: addr,
            sse_keep_alive: None,
            ..Default::default()
        }),
        ..Default::default()
    };

    tokio::spawn(async move {
        // A new handler per request — the whole point of this file.
        let _ = rusty_mcp::serve(|| Ok(PagedServer::new()), config).await;
    });

    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return format!("http://{addr}/mcp");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("server at {addr} never became ready");
}

async fn connect(url: &str) -> rmcp::service::RunningService<rmcp::RoleClient, ClientInfo> {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url.to_string()),
    );

    ClientInfo::default()
        .serve_with_lifecycle(
            transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .expect("client should connect over http")
}

#[tokio::test]
async fn paging_walks_the_whole_list_across_separate_requests() {
    let url = spawn_server().await;
    let client = connect(&url).await;

    let mut uris = Vec::new();
    let mut pages = 0;
    let mut cursor = None;

    loop {
        let page = client
            .list_resources(Some(
                PaginatedRequestParams::default().with_cursor(cursor.clone()),
            ))
            .await
            .expect("resources/list over http");

        pages += 1;
        uris.extend(page.resources.iter().map(|r| r.uri.clone()));

        // Every page, not just the first — a client caching page two needs the
        // hint as much as one caching page one.
        assert!(page.ttl_ms.is_some(), "page {pages} has no ttlMs");
        assert!(page.cache_scope.is_some(), "page {pages} has no cacheScope");

        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    assert_eq!(pages, 3, "25 entries at 10 a page");
    assert_eq!(uris.len(), RESOURCE_COUNT);

    let unique: std::collections::BTreeSet<_> = uris.iter().collect();
    assert_eq!(unique.len(), RESOURCE_COUNT, "nothing served twice");

    let mut sorted = uris.clone();
    sorted.sort();
    assert_eq!(uris, sorted, "pages arrive in order");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn a_cursor_works_on_a_connection_that_never_saw_the_first_page() {
    // The load-balancer case, made explicit: take a cursor on one connection
    // and spend it on another. Statelessness means this has to work.
    let url = spawn_server().await;

    let first = connect(&url).await;
    let cursor = first
        .list_resources(None)
        .await
        .expect("first page")
        .next_cursor
        .expect("more to come");
    first.cancel().await.expect("cancel first");

    let second = connect(&url).await;
    let page = second
        .list_resources(Some(
            PaginatedRequestParams::default().with_cursor(Some(cursor)),
        ))
        .await
        .expect("second page on a fresh connection");

    assert_eq!(page.resources.len(), PAGE_SIZE);
    assert_eq!(
        page.resources.first().map(|r| r.uri.as_str()),
        Some("mem://r010"),
        "the walk resumes where the other connection left off"
    );

    second.cancel().await.expect("cancel second");
}

#[tokio::test]
async fn a_forged_cursor_is_rejected_over_the_wire() {
    let url = spawn_server().await;
    let client = connect(&url).await;

    let err = client
        .list_resources(Some(
            PaginatedRequestParams::default().with_cursor(Some("not-a-cursor!!".to_string())),
        ))
        .await
        .expect_err("should be rejected");

    // -32602. A client that has lost its place is told so rather than handed a
    // silently empty page.
    assert!(
        err.to_string().contains("cursor"),
        "expected a cursor error, got {err}"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn completion_answers_over_the_wire() {
    let url = spawn_server().await;
    let client = connect(&url).await;

    let prompt = client
        .complete_prompt_argument("explain-error", "language", "r", None)
        .await
        .expect("completion/complete for a prompt");
    assert_eq!(
        prompt.values,
        vec!["Rust", "ruby"],
        "case-insensitive prefix"
    );
    assert_eq!(prompt.total, Some(2));
    assert_eq!(prompt.has_more, Some(false));

    let resource = client
        .complete_resource_argument("db://tables/{table}", "table", "", None)
        .await
        .expect("completion/complete for a resource template");
    assert_eq!(resource.values, vec!["orders", "users"]);

    // Speculative asks are ordinary. An unregistered reference is an empty
    // list, not an error the client has to special-case.
    let unknown = client
        .complete_prompt_argument("no-such-prompt", "language", "", None)
        .await
        .expect("no error for an unknown reference");
    assert!(unknown.values.is_empty());

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn templates_paginate_independently_of_resources() {
    let url = spawn_server().await;
    let client = connect(&url).await;

    let templates = client
        .list_resource_templates(None)
        .await
        .expect("resources/templates/list");

    assert_eq!(templates.resource_templates.len(), 1);
    assert!(
        templates.next_cursor.is_none(),
        "one template fits in one page"
    );
    assert!(templates.ttl_ms.is_some());

    client.cancel().await.expect("cancel");
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_paginate_across_separate_requests() {
    // `rmcp`'s `#[tool_handler]` returns every tool in one response and never
    // sets a cursor, which is why `forward_tool_methods!` replaces it. This is
    // the same walk the resources test does, over the tool sequence.
    let url = spawn_server().await;
    let client = connect(&url).await;

    let mut names = Vec::new();
    let mut pages = 0;
    let mut cursor = None;

    loop {
        let page = client
            .list_tools(Some(
                PaginatedRequestParams::default().with_cursor(cursor.clone()),
            ))
            .await
            .expect("tools/list over http");

        pages += 1;
        names.extend(page.tools.iter().map(|t| t.name.to_string()));
        assert!(page.ttl_ms.is_some(), "page {pages} has no ttlMs");
        assert!(page.cache_scope.is_some(), "page {pages} has no cacheScope");

        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    assert_eq!(pages, 3, "six tools at two a page");
    assert_eq!(
        names,
        ["alpha", "bravo", "charlie", "xray", "yankee", "zulu"],
        "every tool exactly once, in key order"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test(flavor = "multi_thread")]
async fn calling_a_tool_still_works_without_the_attribute_macro() {
    // `forward_tool_methods!` generates `call_tool` and `get_tool` as well as
    // the paginated list. Dropping `#[tool_handler]` must not cost dispatch.
    let url = spawn_server().await;
    let client = connect(&url).await;

    let result = client
        .call_tool(rmcp::model::CallToolRequestParams::new("alpha"))
        .await
        .expect("tools/call");

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text");
    assert_eq!(text, "a");

    client.cancel().await.expect("cancel");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tools_cursor_is_rejected_by_the_other_sequences() {
    // Four independent lists share one cursor format; the sequence tag is what
    // stops a cursor from one seeking into another.
    let url = spawn_server().await;
    let client = connect(&url).await;

    let cursor = client
        .list_tools(None)
        .await
        .expect("tools/list")
        .next_cursor
        .expect("more to come");

    let err = client
        .list_resources(Some(
            PaginatedRequestParams::default().with_cursor(Some(cursor)),
        ))
        .await
        .expect_err("a tools cursor must not work on resources");
    assert!(err.to_string().contains("cursor"), "got {err}");

    client.cancel().await.expect("cancel");
}
