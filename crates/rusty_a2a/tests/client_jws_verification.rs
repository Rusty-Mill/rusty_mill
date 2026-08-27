//! Covers client-side JWS Agent Card signature verification (spec Section
//! 8.4) wired into `discover_and_verify` on `A2aClient`, `RestClient`, and
//! `GrpcClient`: a correctly-signed card verifies and discovery succeeds,
//! while an unsigned card, a card signed only by an untrusted key, and
//! (conversely) a card carrying multiple signatures where only one is
//! trusted are all handled correctly.
//!
//! Tampering detection and cross-algorithm rejection are already covered
//! by `src/signing.rs`'s own unit tests at the primitive level; this file
//! only covers the new client-side wiring.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{A2aClient, ClientError, GrpcClient, RestClient};
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::signing::{AgentCardSigningExt, SigningKey};
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

/// Spins up JSON-RPC+REST (one port) and gRPC (another port) interfaces,
/// serving whatever `AgentCard` `build_card` produces from the two
/// interface URLs, and returns the HTTP base URL.
async fn spawn_test_server(build_card: impl FnOnce(String, String) -> AgentCard) -> String {
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();
    let http_base_url = format!("http://{http_addr}");

    let grpc_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    drop(grpc_listener);
    let grpc_url = format!("http://{grpc_addr}");

    let card = build_card(http_base_url.clone(), grpc_url);
    let services = AgentServer::new(card, Arc::new(EchoAgent)).build();

    let http_services = services.clone();
    tokio::spawn(async move {
        axum::serve(http_listener, http_services.router()).await.unwrap();
    });
    tokio::spawn(async move {
        services.serve_grpc(grpc_addr).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    http_base_url
}

fn signed_card(http_url: String, grpc_url: String, key: &SigningKey, kid: &str) -> AgentCard {
    let card = AgentCard::new(
        "JWS Verify Test Agent",
        "An A2A agent used for rusty_a2a's client-side JWS verification tests.",
        "0.0.0",
        AgentInterface::json_rpc(http_url.clone()),
    )
    .with_interface(AgentInterface::http_json(http_url))
    .with_interface(AgentInterface::grpc(grpc_url));
    card.signed(key, Some(kid)).expect("sign_agent_card")
}

#[tokio::test]
async fn correctly_signed_card_verifies_across_all_three_clients() {
    let key = SigningKey::generate_es256().expect("generate_es256");
    let verifying_key = key.verifying_key();
    let trusted = vec![verifying_key];

    let base_url =
        spawn_test_server(|http_url, grpc_url| signed_card(http_url, grpc_url, &key, "key-1")).await;

    let (_client, card) = A2aClient::discover_and_verify(&base_url, &trusted)
        .await
        .expect("A2aClient::discover_and_verify should succeed");
    assert_eq!(card.signatures.len(), 1);

    let (_client, card) = RestClient::discover_and_verify(&base_url, &trusted)
        .await
        .expect("RestClient::discover_and_verify should succeed");
    assert_eq!(card.signatures.len(), 1);

    let (_client, card) = GrpcClient::discover_and_verify(&base_url, &trusted)
        .await
        .expect("GrpcClient::discover_and_verify should succeed");
    assert_eq!(card.signatures.len(), 1);
}

#[tokio::test]
async fn discover_and_verify_rejects_an_unsigned_card() {
    let key = SigningKey::generate_es256().expect("generate_es256");
    let trusted = vec![key.verifying_key()];

    let base_url = spawn_test_server(|http_url, grpc_url| {
        AgentCard::new(
            "JWS Verify Test Agent (unsigned)",
            "An A2A agent used for rusty_a2a's client-side JWS verification tests.",
            "0.0.0",
            AgentInterface::json_rpc(http_url.clone()),
        )
        .with_interface(AgentInterface::http_json(http_url))
        .with_interface(AgentInterface::grpc(grpc_url))
    })
    .await;

    let err = A2aClient::discover_and_verify(&base_url, &trusted)
        .await
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(err, ClientError::AgentCardSignatureInvalid(_)));

    // Plain `discover` (no verification) must still work against the same
    // unsigned card - proving the rejection above is specific to
    // `discover_and_verify`, not a server-side problem.
    A2aClient::discover(&base_url)
        .await
        .expect("plain discover should still succeed");
}

#[tokio::test]
async fn discover_and_verify_rejects_a_card_signed_only_by_an_untrusted_key() {
    let signing_key = SigningKey::generate_es256().expect("generate_es256");
    let untrusted_key = SigningKey::generate_es256().expect("generate_es256");
    let trusted = vec![untrusted_key.verifying_key()];

    let base_url =
        spawn_test_server(|http_url, grpc_url| signed_card(http_url, grpc_url, &signing_key, "key-1")).await;

    let err = A2aClient::discover_and_verify(&base_url, &trusted)
        .await
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(err, ClientError::AgentCardSignatureInvalid(_)));
}

#[tokio::test]
async fn discover_and_verify_succeeds_when_any_trusted_key_matches_one_of_multiple_signatures() {
    let key_a = SigningKey::generate_es256().expect("generate_es256");
    let key_b = SigningKey::generate_ed25519().expect("generate_ed25519");
    // Only `key_b` is trusted by the client, even though the card is
    // signed by both (spec Section 8.4 allows multiple coexisting
    // signatures, e.g. for key rotation, distinguished by `kid`).
    let trusted = vec![key_b.verifying_key()];

    let base_url = spawn_test_server(|http_url, grpc_url| {
        let card = AgentCard::new(
            "JWS Verify Test Agent (multi-sig)",
            "An A2A agent used for rusty_a2a's client-side JWS verification tests.",
            "0.0.0",
            AgentInterface::json_rpc(http_url.clone()),
        )
        .with_interface(AgentInterface::http_json(http_url))
        .with_interface(AgentInterface::grpc(grpc_url));
        let signed_once = card.signed(&key_a, Some("key-a")).expect("sign with key_a");
        signed_once
            .signed(&key_b, Some("key-b"))
            .expect("sign with key_b")
    })
    .await;

    let (_client, card) = A2aClient::discover_and_verify(&base_url, &trusted)
        .await
        .expect("A2aClient::discover_and_verify should succeed via key_b");
    assert_eq!(card.signatures.len(), 2);
}
