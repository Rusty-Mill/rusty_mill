//! How large an input the server accepts, and what it says when it will not.
//!
//! Multimodal messages carrying inline `base64` content are part of what ACP
//! is for, and axum's default body limit of 2 MiB left about 1.5 MiB of actual
//! bytes after the encoding — under one photo from a phone. The limit was also
//! not reachable from the builder, so it was not a default to raise but a
//! ceiling with no handle on it.
//!
//! Sizes here are of the *encoded* body, which is what the limit governs. The
//! raw payloads are three quarters of that, which is why a test asking for
//! "just over the limit" builds a payload smaller than the limit itself.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;

use rusty_acp::server::store::{InMemoryStore, Store};
use rusty_acp::server::{agent_fn, AcpServer, RunContext, DEFAULT_MAX_REQUEST_BYTES};
use rusty_acp::types::{AgentManifest, AgentName, MessagePart, Role};

/// A server accepting any content type, so size is the only thing under test.
async fn server_accepting(limit: Option<usize>) -> String {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::default());
    let sink = agent_fn(
        AgentManifest::new(AgentName::new("sink").unwrap(), "Accepts anything")
            .with_input_content_types(["*/*"]),
        |ctx: RunContext| async move { ctx.reply_text("ok").await.map(|_| ()) },
    );

    let mut builder = AcpServer::builder().agent(sink).store(store).base_url("http://acp.example");
    if let Some(limit) = limit {
        builder = builder.max_request_bytes(limit);
    }
    let router = builder.build().unwrap().into_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

/// Submit a run whose input is one inline artifact of `raw_bytes` raw bytes.
async fn submit_artifact(base_url: &str, raw_bytes: usize) -> reqwest::Response {
    let part =
        MessagePart::binary_artifact("blob", "application/octet-stream", &vec![0u8; raw_bytes][..]);
    let body = serde_json::json!({
        "agent_name": "sink",
        "mode": "async",
        "input": [rusty_acp::types::Message::new(Role::User, [part])],
    });
    reqwest::Client::new()
        .post(format!("{base_url}/runs"))
        .json(&body)
        .send()
        .await
        .expect("the request must reach the server")
}

/// The case the old ceiling rejected: an ordinary photo-sized artifact.
///
/// 3 MiB raw is about 4 MiB encoded — over axum's 2 MiB default and inside the
/// 8 MiB this crate now sets.
#[tokio::test]
async fn an_ordinary_photo_sized_artifact_is_accepted() {
    let base_url = server_accepting(None).await;

    let response = submit_artifact(&base_url, 3 * 1024 * 1024).await;

    assert!(
        response.status().is_success(),
        "a 3 MiB artifact was refused with {}: {}",
        response.status(),
        response.text().await.unwrap_or_default()
    );
}

/// The limit is still a limit. A server that buffers whatever it is sent has a
/// memory-exhaustion path that costs an attacker nothing, so the answer is a
/// bigger default rather than no default.
///
/// A guard rather than a discriminator: this passed before the change too,
/// because axum's own 2 MiB refused the same body. It is here to fail if the
/// ceiling is ever raised to `usize::MAX` or dropped altogether, which nothing
/// else would catch.
#[tokio::test]
async fn something_past_the_limit_is_still_refused() {
    let base_url = server_accepting(None).await;

    // Comfortably past 8 MiB once encoded.
    let response = submit_artifact(&base_url, DEFAULT_MAX_REQUEST_BYTES).await;

    assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
}

/// The 413 names the limit and the way around it.
///
/// axum's own message describes its internals — "Failed to buffer the request
/// body" — and mentions neither. A caller that cannot tell how much too large
/// it was has nothing to act on.
#[tokio::test]
async fn the_refusal_says_what_to_do_about_it() {
    let limit = 64 * 1024;
    let base_url = server_accepting(Some(limit)).await;

    let response = submit_artifact(&base_url, limit).await;
    assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);

    let body = response.text().await.unwrap();
    assert!(body.contains(&limit.to_string()), "the limit is not in the message: {body}");
    assert!(body.contains("content_url"), "the way around it is not in the message: {body}");
    assert!(!body.contains("Failed to buffer"), "axum's own rejection reached the caller: {body}");
}

/// The builder actually moves the ceiling, in both directions.
#[tokio::test]
async fn the_limit_is_configurable() {
    // A raw payload that encodes to something between the two limits.
    let raw = 128 * 1024;

    let tight = server_accepting(Some(64 * 1024)).await;
    assert_eq!(
        submit_artifact(&tight, raw).await.status(),
        reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        "a lowered limit was not applied"
    );

    let generous = server_accepting(Some(4 * 1024 * 1024)).await;
    assert!(
        submit_artifact(&generous, raw).await.status().is_success(),
        "a raised limit was not applied"
    );
}

/// The limit guards every endpoint, not only the one expected to carry a large
/// body. A limit that covers the endpoint you thought of is not a limit.
#[tokio::test]
async fn the_limit_covers_every_endpoint() {
    let limit = 64 * 1024;
    let base_url = server_accepting(Some(limit)).await;

    let oversized = serde_json::json!({ "junk": "x".repeat(limit * 2) });
    let response = reqwest::Client::new()
        // Resume, which takes a body and is not the submission endpoint.
        .post(format!("{base_url}/runs/{}", uuid::Uuid::new_v4()))
        .json(&oversized)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
}

/// A malformed body is still reported the way it always was.
///
/// Only the one rejection this crate has something to add to is rewritten;
/// rewriting the rest would be a second, unrelated change to the error surface.
#[tokio::test]
async fn a_malformed_body_is_untouched() {
    let base_url = server_accepting(None).await;

    let response = reqwest::Client::new()
        .post(format!("{base_url}/runs"))
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body = response.text().await.unwrap();
    assert!(!body.contains("content_url"), "a parse failure got the too-large advice: {body}");
}
