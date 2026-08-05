//! End-to-end resource-server tests over a real socket.
//!
//! These drive the wire contract a client actually depends on: the challenge
//! headers, the discovery document, and which tokens get in.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use rusty_mcp::{
    HttpConfig, ServerConfig, Transport,
    auth::{AuthConfig, StaticTokenValidator, TokenError, TokenValidator, VerifiedToken},
};
use schemars::JsonSchema;
use serde::Deserialize;

const RESOURCE: &str = "https://mcp.example.com/mcp";
const METADATA_PATH: &str = "/.well-known/oauth-protected-resource/mcp";

/// Arguments for the `echo` tool.
#[derive(Debug, Deserialize, JsonSchema)]
struct EchoArgs {
    /// Text to echo back.
    message: String,
}

/// Minimal protected server.
#[derive(Clone)]
struct EchoServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl EchoServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Echo the message back.")]
    async fn echo(&self, Parameters(EchoArgs { message }): Parameters<EchoArgs>) -> String {
        message
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for EchoServer {
    fn get_info(&self) -> ServerInfo {
        rusty_mcp::server_info(
            "echo-server",
            "0.1.0",
            ServerCapabilities::builder().enable_tools().build(),
        )
    }
}

/// A validator whose backing store is always down.
struct BrokenValidator;

impl TokenValidator for BrokenValidator {
    fn validate<'a>(&'a self, _token: &'a str) -> rusty_mcp::auth::ValidateFuture<'a> {
        Box::pin(std::future::ready(Err(TokenError::Unavailable(
            "introspection endpoint unreachable".to_string(),
        ))))
    }
}

fn default_validator() -> StaticTokenValidator {
    StaticTokenValidator::new()
        .with_token(
            "good",
            VerifiedToken::new([RESOURCE])
                .with_scopes(["mcp:read"])
                .with_subject("user-1"),
        )
        .with_token(
            "thin",
            // Valid and correctly audienced, but underprivileged.
            VerifiedToken::new([RESOURCE]).with_scopes(["mcp:ping"]),
        )
        .with_token(
            "foreign",
            // Minted for a different resource server entirely.
            VerifiedToken::new(["https://other.example.com/mcp"]).with_scopes(["mcp:read"]),
        )
}

async fn spawn(auth: AuthConfig) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let config = ServerConfig {
        transport: Transport::Http(HttpConfig {
            bind: addr,
            sse_keep_alive: None,
            auth: Some(Arc::new(auth)),
            ..Default::default()
        }),
        ..Default::default()
    };

    tokio::spawn(async move {
        let _ = rusty_mcp::serve(|| Ok(EchoServer::new()), config).await;
    });

    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return addr;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("server never became ready");
}

async fn spawn_default() -> SocketAddr {
    let auth = AuthConfig::new(RESOURCE, Arc::new(default_validator()))
        .expect("valid resource")
        .with_authorization_servers(["https://auth.example.com"])
        .with_scopes_supported(["mcp:read"])
        .with_required_scopes(["mcp:read"]);
    spawn(auth).await
}

/// POST a `tools/list` with the given bearer token, if any.
async fn tools_list(addr: SocketAddr, token: Option<&str>) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/list")
        .body(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{
                 "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                 "io.modelcontextprotocol/clientCapabilities":{}}}}"#,
        );

    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    request.send().await.expect("request reaches the server")
}

fn www_authenticate(response: &reqwest::Response) -> String {
    response
        .headers()
        .get("www-authenticate")
        .expect("a challenge header")
        .to_str()
        .expect("ascii")
        .to_string()
}

#[tokio::test]
async fn unauthenticated_requests_get_a_challenge_pointing_at_the_metadata() {
    let addr = spawn_default().await;
    let response = tools_list(addr, None).await;

    assert_eq!(response.status(), 401);

    // Without this header the client has no way to discover where to
    // authenticate, which is the whole point of the 401.
    let challenge = www_authenticate(&response);
    assert!(challenge.starts_with("Bearer"), "{challenge}");
    assert!(
        challenge.contains(&format!(
            "resource_metadata=\"https://mcp.example.com{METADATA_PATH}\""
        )),
        "{challenge}"
    );
    // Scope guidance, so the client asks for the right thing first time.
    assert!(challenge.contains("scope=\"mcp:read\""), "{challenge}");
    // RFC 6750: a request with no credentials carries no error code.
    assert!(!challenge.contains("error="), "{challenge}");
}

#[tokio::test]
async fn a_valid_token_reaches_the_handler() {
    let addr = spawn_default().await;
    let response = tools_list(addr, Some("good")).await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["result"]["tools"][0]["name"], "echo");
}

#[tokio::test]
async fn a_token_for_another_resource_is_rejected() {
    // The confused-deputy case the spec's "MUST NOT accept or transit any
    // other tokens" is aimed at: a genuine token, just not ours.
    let addr = spawn_default().await;
    let response = tools_list(addr, Some("foreign")).await;

    assert_eq!(response.status(), 401);
    let challenge = www_authenticate(&response);
    assert!(challenge.contains("error=\"invalid_token\""), "{challenge}");
}

#[tokio::test]
async fn an_unknown_token_is_rejected() {
    let addr = spawn_default().await;
    let response = tools_list(addr, Some("nonsense")).await;

    assert_eq!(response.status(), 401);
    assert!(www_authenticate(&response).contains("error=\"invalid_token\""));
}

#[tokio::test]
async fn insufficient_scope_is_403_not_401() {
    let addr = spawn_default().await;
    let response = tools_list(addr, Some("thin")).await;

    // 403, because re-authenticating would not help — the client needs a
    // *broader* token, not a new one.
    assert_eq!(response.status(), 403);

    let challenge = www_authenticate(&response);
    assert!(
        challenge.contains("error=\"insufficient_scope\""),
        "{challenge}"
    );
    assert!(challenge.contains("scope=\"mcp:read\""), "{challenge}");
    assert!(challenge.contains("resource_metadata="), "{challenge}");
}

#[tokio::test]
async fn a_malformed_authorization_header_is_400() {
    let addr = spawn_default().await;

    let response = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header("Content-Type", "application/json")
        .header("Authorization", "Basic dXNlcjpwdw==")
        .body("{}")
        .send()
        .await
        .expect("request");

    assert_eq!(response.status(), 400);
    assert!(www_authenticate(&response).contains("error=\"invalid_request\""));
}

#[tokio::test]
async fn the_metadata_document_is_served_without_a_token() {
    let addr = spawn_default().await;

    // Must be reachable unauthenticated, or discovery deadlocks.
    let response = reqwest::Client::new()
        .get(format!("http://{addr}{METADATA_PATH}"))
        .send()
        .await
        .expect("request");

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("json");

    assert_eq!(body["resource"], RESOURCE);
    assert_eq!(body["authorization_servers"][0], "https://auth.example.com");
    assert_eq!(body["scopes_supported"][0], "mcp:read");
    assert_eq!(body["bearer_methods_supported"][0], "header");
}

#[tokio::test]
async fn a_validator_outage_is_503_not_401() {
    // A 401 would tell the client its token is bad and send the user through a
    // pointless re-login, when the real fault is on the server side.
    let auth = AuthConfig::new(RESOURCE, Arc::new(BrokenValidator))
        .expect("valid resource")
        .with_required_scopes(["mcp:read"]);
    let addr = spawn(auth).await;

    let response = tools_list(addr, Some("good")).await;

    assert_eq!(response.status(), 503);
    assert!(response.headers().get("www-authenticate").is_none());
}

#[tokio::test]
async fn tools_can_read_the_verified_token() {
    // The layer puts the token in the request extensions, which the transport
    // forwards to handlers as `http::request::Parts` — this is what makes
    // per-tool scope checks possible.
    #[derive(Clone)]
    struct WhoAmIServer {
        tool_router: ToolRouter<Self>,
    }

    #[tool_router(router = tool_router)]
    impl WhoAmIServer {
        #[tool(description = "Return the authenticated subject.")]
        async fn whoami(
            &self,
            ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
        ) -> Result<String, rmcp::model::ErrorData> {
            let subject = ctx
                .extensions
                .get::<http::request::Parts>()
                .and_then(|parts| parts.extensions.get::<VerifiedToken>())
                .and_then(|token| token.subject.clone())
                .ok_or_else(|| {
                    rmcp::model::ErrorData::invalid_request("no authenticated subject", None)
                })?;
            Ok(subject)
        }
    }

    #[tool_handler(router = self.tool_router)]
    impl ServerHandler for WhoAmIServer {
        fn get_info(&self) -> ServerInfo {
            rusty_mcp::server_info(
                "whoami-server",
                "0.1.0",
                ServerCapabilities::builder().enable_tools().build(),
            )
        }
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let auth = AuthConfig::new(RESOURCE, Arc::new(default_validator()))
        .expect("valid resource")
        .with_required_scopes(["mcp:read"]);

    let config = ServerConfig {
        transport: Transport::Http(HttpConfig {
            bind: addr,
            sse_keep_alive: None,
            auth: Some(Arc::new(auth)),
            ..Default::default()
        }),
        ..Default::default()
    };
    tokio::spawn(async move {
        let _ = rusty_mcp::serve(
            || {
                Ok(WhoAmIServer {
                    tool_router: WhoAmIServer::tool_router(),
                })
            },
            config,
        )
        .await;
    });
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let response = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", "whoami")
        .header("Authorization", "Bearer good")
        .body(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
                 "name":"whoami","arguments":{},"_meta":{
                 "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                 "io.modelcontextprotocol/clientCapabilities":{}}}}"#,
        )
        .send()
        .await
        .expect("request");

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["result"]["content"][0]["text"], "user-1");
}
