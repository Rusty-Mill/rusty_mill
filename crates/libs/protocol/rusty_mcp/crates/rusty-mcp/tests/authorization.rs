#![allow(clippy::unwrap_used)]
//! End-to-end resource-server tests over a real socket.
//!
//! These drive the wire contract a client actually depends on: the challenge
//! headers, the discovery document, and which tokens get in. The MCP server
//! behind the layer is the real one (`rusty_mcp_server` through
//! `rusty_mcp_axum`), mounted the way a gateway mounts it.

mod support;

use std::{net::SocketAddr, sync::Arc};

use axum::{Json, Router, routing::get};
use rusty_mcp::auth::{
    AuthConfig, ProtectedResourceMetadata, RequireAuthLayer, StaticTokenValidator, TokenError,
    TokenValidator, VerifiedToken,
};
use rusty_mcp_server::ChangeBroadcaster;
use support::{PATH, call, mcp_router, post, serve, tools_list};

const RESOURCE: &str = "https://mcp.example.com/mcp";
const METADATA_PATH: &str = "/.well-known/oauth-protected-resource/mcp";

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

/// The guarded MCP route with the metadata document beside it, unguarded.
async fn spawn(auth: AuthConfig) -> SocketAddr {
    let metadata = ProtectedResourceMetadata::from_config(&auth);
    let metadata_path = auth.metadata_path();
    let app = Router::new()
        .route(
            &metadata_path,
            get(move || {
                let metadata = metadata.clone();
                async move { Json(metadata) }
            }),
        )
        .nest_service(
            PATH,
            mcp_router(&ChangeBroadcaster::new()).layer(RequireAuthLayer::new(auth)),
        );
    serve(app).await
}

async fn spawn_default() -> SocketAddr {
    let auth = AuthConfig::new(RESOURCE, Arc::new(default_validator()))
        .expect("valid resource")
        .with_authorization_servers(["https://auth.example.com"])
        .with_scopes_supported(["mcp:read"])
        .with_required_scopes(["mcp:read"]);
    spawn(auth).await
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
    let response = post(addr, "tools/list", None, tools_list(), None).await;

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
    let response = post(addr, "tools/list", None, tools_list(), Some("good")).await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("json");
    let names: Vec<_> = body["result"]["tools"]
        .as_array()
        .expect("a tool list")
        .iter()
        .map(|t| t["name"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(names.iter().any(|n| n == "echo"), "{names:?}");
}

#[tokio::test]
async fn a_token_for_another_resource_is_rejected() {
    // The confused-deputy case the spec's "MUST NOT accept or transit any
    // other tokens" is aimed at: a genuine token, just not ours.
    let addr = spawn_default().await;
    let response = post(addr, "tools/list", None, tools_list(), Some("foreign")).await;

    assert_eq!(response.status(), 401);
    let challenge = www_authenticate(&response);
    assert!(challenge.contains("error=\"invalid_token\""), "{challenge}");
}

#[tokio::test]
async fn an_unknown_token_is_rejected() {
    let addr = spawn_default().await;
    let response = post(addr, "tools/list", None, tools_list(), Some("nonsense")).await;

    assert_eq!(response.status(), 401);
    assert!(www_authenticate(&response).contains("error=\"invalid_token\""));
}

#[tokio::test]
async fn insufficient_scope_is_403_not_401() {
    let addr = spawn_default().await;
    let response = post(addr, "tools/list", None, tools_list(), Some("thin")).await;

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
        .post(format!("http://{addr}{PATH}"))
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

    let response = post(addr, "tools/list", None, tools_list(), Some("good")).await;

    assert_eq!(response.status(), 503);
    assert!(response.headers().get("www-authenticate").is_none());
}

#[tokio::test]
async fn tools_can_read_the_verified_token() {
    // The layer puts the token in the request extensions; the application's
    // `principal_of` hands its subject to the handlers as `Caller::principal`
    // — this is what makes per-tool checks possible.
    let addr = spawn_default().await;
    let response = post(
        addr,
        "tools/call",
        Some("whoami"),
        call("whoami", "{}"),
        Some("good"),
    )
    .await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["result"]["content"][0]["text"], "user-1");
}
