//! Covers `AgentServer::with_mtls_header`: satisfying an `mtls` security
//! scheme via a header/gRPC-metadata entry a TLS-terminating reverse
//! proxy sets to report whether it already verified the client's
//! certificate - the closest this crate's own (TLS-less) servers can get
//! to enforcing `mtls`, since they never see the client certificate
//! directly (see the `server::auth` module docs).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{A2aClient, ClientError};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{
    AgentExecutor, AgentServer, AuthContext, AuthVerifier, Credentials, EventSink, RequestContext,
};
use rusty_a2a::types::{
    AgentCard, AgentInterface, Message, MutualTlsSecurityScheme, SecurityRequirement, SecurityScheme,
    StringList, TaskState,
};

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

/// Mimics an `nginx`-style proxy convention: the `ssl-client-verify`
/// header/metadata entry must read exactly `"SUCCESS"`.
struct MtlsProxyVerifier;

#[async_trait]
impl AuthVerifier for MtlsProxyVerifier {
    async fn verify(
        &self,
        _requirement: &SecurityRequirement,
        credentials: &Credentials,
    ) -> Result<AuthContext> {
        match credentials.0.get("mtls") {
            Some(v) if v == "SUCCESS" => Ok(AuthContext::new("proxy-verified-client")),
            _ => Err(A2aError::Unauthenticated(
                "client certificate not verified by the proxy".to_string(),
            )),
        }
    }
}

fn mtls_scheme() -> SecurityScheme {
    SecurityScheme::MutualTls {
        mtls_security_scheme: MutualTlsSecurityScheme { description: None },
    }
}

fn mtls_requirement() -> SecurityRequirement {
    SecurityRequirement {
        schemes: HashMap::from([("mtls".to_string(), StringList { list: Vec::new() })]),
    }
}

async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut card = AgentCard::new(
        "mTLS Proxy Header Test Agent",
        "An A2A agent used for rusty_a2a's mtls-via-proxy-header tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    card.security_schemes.insert("mtls".to_string(), mtls_scheme());
    card.security_requirements = vec![mtls_requirement()];

    let server = AgentServer::new(card, Arc::new(EchoAgent))
        .with_auth_verifier(Arc::new(MtlsProxyVerifier))
        .with_mtls_header("ssl-client-verify");
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[tokio::test]
async fn without_with_mtls_header_configured_mtls_is_never_satisfiable() {
    // No `.with_mtls_header(...)` at all - even a request carrying exactly
    // the header a proxy would set must still be rejected, proving the
    // scheme really is inert by default (the behavior documented before
    // this feature existed), not just untested by the happy-path server.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut card = AgentCard::new(
        "mTLS Without Header Config Test Agent",
        "An A2A agent used for rusty_a2a's mtls-via-proxy-header tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    card.security_schemes.insert("mtls".to_string(), mtls_scheme());
    card.security_requirements = vec![mtls_requirement()];

    let server = AgentServer::new(card, Arc::new(EchoAgent)).with_auth_verifier(Arc::new(MtlsProxyVerifier));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{base_url}/"))
        .header("A2A-Version", "1.0")
        .header("ssl-client-verify", "SUCCESS")
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "SendMessage",
            "params": {"message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}}
        }))
        .send()
        .await
        .expect("POST /");
    let body: serde_json::Value = resp.json().await.expect("response body");
    assert!(
        body.get("error").is_some(),
        "expected a JSON-RPC error, got {body:?}"
    );
}

#[tokio::test]
async fn json_rpc_accepts_a_verified_proxy_header_and_rejects_everything_else() {
    let base_url = spawn_test_server().await;

    let verified = A2aClient::new(format!("{base_url}/")).with_bearer_token("irrelevant");
    // `A2aClient` has no generic "set an arbitrary header" hook, so drive
    // this one with a bare `reqwest::Client` to control the exact header.
    let http = reqwest::Client::new();

    let accepted = http
        .post(format!("{base_url}/"))
        .header("A2A-Version", "1.0")
        .header("ssl-client-verify", "SUCCESS")
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "SendMessage",
            "params": {"message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}}
        }))
        .send()
        .await
        .expect("POST / (verified)");
    let accepted_body: serde_json::Value = accepted.json().await.expect("response body");
    assert!(
        accepted_body.get("result").is_some(),
        "expected success, got {accepted_body:?}"
    );

    let wrong_value = http
        .post(format!("{base_url}/"))
        .header("A2A-Version", "1.0")
        .header("ssl-client-verify", "FAILED")
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "SendMessage",
            "params": {"message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}}
        }))
        .send()
        .await
        .expect("POST / (proxy reports failure)");
    let wrong_value_body: serde_json::Value = wrong_value.json().await.expect("response body");
    assert!(wrong_value_body.get("error").is_some());

    let missing = http
        .post(format!("{base_url}/"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "SendMessage",
            "params": {"message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}}
        }))
        .send()
        .await
        .expect("POST / (no header at all)");
    let missing_body: serde_json::Value = missing.json().await.expect("response body");
    assert!(missing_body.get("error").is_some());

    // `verified` (an `A2aClient` with an irrelevant bearer token and no
    // `ssl-client-verify` header) must fail the same way `missing` does -
    // this just confirms the client variable above compiles/behaves
    // unsurprisingly, not a new code path.
    let err = verified
        .send_message(Message::user_text("hi"), None)
        .await
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::Unauthenticated(_)) => {}
        other => panic!("expected Unauthenticated, got {other:?}"),
    }
}

#[tokio::test]
async fn rest_binding_reads_the_same_configured_header() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let accepted = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .header("ssl-client-verify", "SUCCESS")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /message:send (verified)");
    assert_eq!(accepted.status(), 200);

    let rejected = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /message:send (no header)");
    assert_eq!(rejected.status(), 401);
    let body: serde_json::Value = rejected.json().await.expect("response body");
    assert_eq!(body["error"]["status"], "UNAUTHENTICATED");
}
