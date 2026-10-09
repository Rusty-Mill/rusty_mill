#![allow(clippy::unwrap_used)]
//! `McpClient` against a real server: `rusty_mcp_server` over HTTP in this
//! process, and over stdio as a child process.

use rusty_mcp_client::client::{McpClient, McpClientError, McpServerSpec, McpTransport};
use rusty_mcp_client::proto::{ContentBlock, ResourceContents};
use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{CallToolResult, Prompt, ReadResourceResult, Resource, Tool};
use rusty_mcp_server::{HttpConfig, Server, bind_http};
use rusty_serve::Limits;
use std::sync::Arc;

fn schema() -> Value {
    let mut s = Value::object();
    s.insert("type", "object");
    s
}

fn server() -> Server {
    Server::builder("e2e", "1")
        .page_size(2)
        .tool(Tool::new("add", schema()), |_c, call| {
            let n = |k: &str| {
                call.arguments
                    .as_ref()
                    .and_then(|a| a.get(k))
                    .and_then(Value::as_i64)
            };
            let sum = n("a").unwrap_or(0) + n("b").unwrap_or(0);
            Ok(CallToolResult {
                content: vec![ContentBlock::text(sum.to_string())],
                ..CallToolResult::default()
            })
        })
        .tool(Tool::new("two", schema()), |_c, _p| {
            Ok(CallToolResult::default())
        })
        .tool(Tool::new("three", schema()), |_c, _p| {
            Ok(CallToolResult::default())
        })
        .prompt(Prompt::new("p"), |_c, _g| Ok(Default::default()))
        .resource(Resource::new("mem://a", "a"), |_c, read| {
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::Text {
                    uri: read.uri,
                    mime_type: None,
                    text: "a".to_owned(),
                    meta: None,
                }],
                ..ReadResourceResult::default()
            })
        })
        .build()
        .unwrap()
}

fn http_spec() -> (McpServerSpec, rusty_serve::ShutdownHandle) {
    let http = bind_http(
        Arc::new(server()),
        "127.0.0.1:0".parse().unwrap(),
        HttpConfig::default(),
        Limits::default(),
    )
    .unwrap();
    let url = format!("http://{}/mcp", http.local_addr().unwrap());
    let stop = http.shutdown_handle().unwrap();
    std::thread::spawn(move || http.run().unwrap());
    (
        McpServerSpec {
            transport: McpTransport::Http,
            url: Some(url),
            ..McpServerSpec::default()
        },
        stop,
    )
}

fn args(a: i64, b: i64) -> Option<serde_json::Map<String, serde_json::Value>> {
    serde_json::json!({ "a": a, "b": b }).as_object().cloned()
}

fn text(r: &rusty_mcp_client::proto::CallToolResult) -> String {
    match &r.content[0] {
        ContentBlock::Text { text, .. } => text.clone(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn http_lists_calls_and_shuts_down() {
    let (spec, stop) = http_spec();
    let client = McpClient::connect("e2e", &spec).await.unwrap();
    assert_eq!(client.name(), "e2e");
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 3, "followed the pages");
    assert_eq!(client.list_prompts().await.unwrap()[0].name, "p");
    assert_eq!(client.list_resources().await.unwrap()[0].uri, "mem://a");
    let r = client.call_tool("add", args(40, 2)).await.unwrap();
    assert_eq!(text(&r), "42");
    tokio::time::timeout(std::time::Duration::from_secs(5), client.shutdown())
        .await
        .expect("shutdown hung")
        .unwrap();
    stop.shutdown();
}

#[tokio::test]
async fn a_server_error_is_a_service_error_and_transient() {
    let (spec, stop) = http_spec();
    let client = McpClient::connect("e2e", &spec).await.unwrap();
    let err = client.call_tool("nope", None).await.unwrap_err();
    assert!(matches!(&err, McpClientError::Service(_)), "{err:?}");
    assert!(err.is_transient());
    stop.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn calls_from_many_tasks_share_one_client() {
    let (spec, stop) = http_spec();
    let client = Arc::new(McpClient::connect("e2e", &spec).await.unwrap());
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let c = Arc::clone(&client);
            tokio::spawn(async move { text(&c.call_tool("add", args(i, 1)).await.unwrap()) })
        })
        .collect();
    for (i, h) in handles.into_iter().enumerate() {
        assert_eq!(h.await.unwrap(), (i + 1).to_string());
    }
    stop.shutdown();
}

/// The OAuth client-credentials fetch runs on the caller's tokio runtime
/// (via `rusty_request`'s `tokio` feature) against a raw token endpoint.
mod oauth {
    use rusty_mcp_client::client_auth::{AuthError, McpAuth, McpAuthSecret, resolve};
    use rusty_mcp_client::client_auth::{ClientIdSecret, ClientSecretSecret};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn declaration(url: String) -> McpAuth {
        McpAuth::OauthClientCredentials {
            token_url: url,
            client_id: ClientIdSecret::Inline {
                client_id: "id".to_owned(),
            },
            client_secret: ClientSecretSecret::Inline {
                client_secret: "shh".to_owned(),
            },
            scope: Some("read".to_owned()),
        }
    }

    /// Serve one canned response and report the request.
    async fn token_endpoint(status: &str, body: &str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let (status, body) = (status.to_owned(), body.to_owned());
        let seen = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = s.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(reply.as_bytes()).await.unwrap();
            request
        });
        (url, seen)
    }

    #[tokio::test]
    async fn a_token_is_fetched_with_basic_auth_and_a_form_body() {
        let (url, seen) =
            token_endpoint("200 OK", r#"{"access_token":"tok","token_type":"Bearer"}"#).await;
        let resolved = resolve(&declaration(url)).await.unwrap();
        assert_eq!(resolved.authorization.as_deref(), Some("Bearer tok"));
        let request = seen.await.unwrap().to_ascii_lowercase();
        // base64("id:shh") = aWQ6c2ho
        assert!(
            request.contains("authorization: basic awq6c2ho"),
            "{request}"
        );
        assert!(
            request.contains("grant_type=client_credentials"),
            "{request}"
        );
        assert!(request.contains("scope=read"), "{request}");
        assert!(
            request.contains("application/x-www-form-urlencoded"),
            "{request}"
        );
    }

    #[tokio::test]
    async fn a_refused_or_malformed_token_is_an_auth_error() {
        let (url, _) = token_endpoint("401 Unauthorized", r#"{"error":"invalid_client"}"#).await;
        let err = resolve(&declaration(url)).await.unwrap_err();
        assert!(
            matches!(&err, AuthError::Oauth { reason, .. } if reason.contains("401")),
            "{err}"
        );

        let (url, _) = token_endpoint("200 OK", r#"{"access_token":"t","token_type":"mac"}"#).await;
        let err = resolve(&declaration(url)).await.unwrap_err();
        assert!(matches!(err, AuthError::OauthResponse { .. }), "{err}");

        let (url, _) = token_endpoint("200 OK", "not json").await;
        assert!(matches!(
            resolve(&declaration(url)).await.unwrap_err(),
            AuthError::OauthResponse { .. }
        ));
    }

    #[tokio::test]
    async fn inline_secrets_still_resolve_without_io() {
        let auth = McpAuth::Bearer {
            token: McpAuthSecret::Inline {
                value: "abc".to_owned(),
            },
        };
        assert_eq!(
            resolve(&auth).await.unwrap().authorization.as_deref(),
            Some("Bearer abc")
        );
    }
}
