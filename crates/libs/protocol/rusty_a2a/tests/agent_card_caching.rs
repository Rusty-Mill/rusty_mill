//! Covers spec Section 8.6.1 (SHOULD): the Agent Card discovery endpoint
//! sends `Cache-Control` and an `ETag`, and honors a conditional-GET
//! `If-None-Match` with a bare `304 Not Modified`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Message, TaskState};

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "Agent Card Caching Test Agent",
        "An A2A agent used for rusty_a2a's Agent Card caching header tests.",
        "1.2.3",
        AgentInterface::json_rpc(base_url.clone()),
    );

    let server = AgentServer::new(card, Arc::new(EchoAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[tokio::test]
async fn agent_card_response_carries_cache_control_and_etag() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .get(format!("{base_url}{}", rusty_a2a::AGENT_CARD_WELL_KNOWN_PATH))
        .send()
        .await
        .expect("GET agent-card.json");
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers().get("cache-control").is_some(),
        "expected a Cache-Control header"
    );
    let etag = resp
        .headers()
        .get("etag")
        .expect("expected an ETag header")
        .to_str()
        .expect("valid ETag header")
        .to_string();
    assert!(
        etag.contains("1.2.3"),
        "expected the ETag to reflect the card's version, got {etag:?}"
    );

    let body: serde_json::Value = resp.json().await.expect("response body");
    assert_eq!(body["name"], "Agent Card Caching Test Agent");
}

#[tokio::test]
async fn matching_if_none_match_gets_a_304_with_no_body() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let first = http
        .get(format!("{base_url}{}", rusty_a2a::AGENT_CARD_WELL_KNOWN_PATH))
        .send()
        .await
        .expect("GET agent-card.json");
    let etag = first
        .headers()
        .get("etag")
        .expect("expected an ETag header")
        .to_str()
        .expect("valid ETag header")
        .to_string();

    let second = http
        .get(format!("{base_url}{}", rusty_a2a::AGENT_CARD_WELL_KNOWN_PATH))
        .header("If-None-Match", &etag)
        .send()
        .await
        .expect("GET agent-card.json (conditional)");
    assert_eq!(second.status(), 304);
    assert_eq!(
        second.headers().get("etag").and_then(|v| v.to_str().ok()),
        Some(etag.as_str())
    );
    let body = second.bytes().await.expect("response body");
    assert!(body.is_empty(), "304 response should have no body");
}

#[tokio::test]
async fn a_stale_if_none_match_still_gets_the_full_card() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .get(format!("{base_url}{}", rusty_a2a::AGENT_CARD_WELL_KNOWN_PATH))
        .header("If-None-Match", "\"some-other-version\"")
        .send()
        .await
        .expect("GET agent-card.json (stale conditional)");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body");
    assert_eq!(body["name"], "Agent Card Caching Test Agent");
}
