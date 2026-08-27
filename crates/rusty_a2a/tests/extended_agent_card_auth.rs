//! Covers spec Section 13.3: `GetExtendedAgentCard` "MUST require
//! authentication" unconditionally (fails closed both with no
//! `AuthVerifier` configured at all, and with a verifier but no
//! `securitySchemes` to authenticate against - neither is "this agent is
//! public", unlike every other operation, where an empty
//! `securityRequirements` legitimately means that), and that its REST
//! response carries `Cache-Control`/`ETag` like the base Agent Card does.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{A2aClient, ClientError};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{
    AgentExecutor, AgentServer, AuthContext, AuthVerifier, Credentials, EventSink, RequestContext,
};
use rusty_a2a::types::{
    AgentCard, AgentInterface, HttpAuthSecurityScheme, Message, SecurityRequirement, SecurityScheme,
    TaskState,
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

fn bearer_scheme() -> SecurityScheme {
    SecurityScheme::HttpAuth {
        http_auth_security_scheme: HttpAuthSecurityScheme {
            description: None,
            scheme: "Bearer".to_string(),
            bearer_format: None,
        },
    }
}

const VALID_TOKEN: &str = "secret-token";

struct BearerVerifier;

#[async_trait]
impl AuthVerifier for BearerVerifier {
    async fn verify(
        &self,
        _requirement: &SecurityRequirement,
        credentials: &Credentials,
    ) -> Result<AuthContext> {
        match credentials.0.get("bearer") {
            Some(token) if token == VALID_TOKEN => Ok(AuthContext::new("test-user")),
            _ => Err(A2aError::Unauthenticated("invalid bearer token".to_string())),
        }
    }
}

/// Spawns a server whose card has `capabilities.extendedAgentCard = true`;
/// `configure_card` runs before the card is cloned for both the base
/// server and the extended card (so a `securitySchemes` entry it adds
/// lands on both, matching how `Engine::extended_card_security_requirements`
/// falls back to the *base* card's `securitySchemes`), and
/// `configure_server` runs after `AgentServer::new` (e.g. to add
/// `with_auth_verifier`).
async fn spawn_test_server(
    configure_card: impl FnOnce(&mut AgentCard),
    configure_server: impl FnOnce(AgentServer) -> AgentServer,
) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut card = AgentCard::new(
        "Extended Card Auth Test Agent",
        "An A2A agent used for rusty_a2a's GetExtendedAgentCard auth tests.",
        "1.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    card.capabilities.extended_agent_card = Some(true);
    configure_card(&mut card);
    let extended_card = card.clone();

    let server =
        configure_server(AgentServer::new(card, Arc::new(EchoAgent)).with_extended_card(extended_card));

    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

fn expect_internal(err: ClientError) {
    match err {
        ClientError::Protocol(A2aError::Internal(_)) => {}
        other => panic!("expected Internal (fail-closed misconfiguration error), got {other:?}"),
    }
}

#[tokio::test]
async fn fails_closed_with_no_auth_verifier_configured_at_all() {
    let base_url = spawn_test_server(
        |card| {
            card.security_schemes
                .insert("bearer".to_string(), bearer_scheme());
        },
        |server| server,
    )
    .await;

    let client = A2aClient::new(format!("{base_url}/"));
    let err = client.get_extended_agent_card().await.unwrap_err();
    expect_internal(err);
}

#[tokio::test]
async fn fails_closed_with_a_verifier_but_no_security_schemes_declared() {
    let base_url = spawn_test_server(
        |_card| {},
        |server| server.with_auth_verifier(Arc::new(BearerVerifier)),
    )
    .await;

    let client = A2aClient::new(format!("{base_url}/"));
    let err = client.get_extended_agent_card().await.unwrap_err();
    expect_internal(err);
}

#[tokio::test]
async fn succeeds_with_both_a_verifier_and_a_declared_scheme() {
    let base_url = spawn_test_server(
        |card| {
            card.security_schemes
                .insert("bearer".to_string(), bearer_scheme());
        },
        |server| server.with_auth_verifier(Arc::new(BearerVerifier)),
    )
    .await;

    let client = A2aClient::new(format!("{base_url}/")).with_bearer_token(VALID_TOKEN);
    let card = client
        .get_extended_agent_card()
        .await
        .expect("get_extended_agent_card");
    assert_eq!(card.name, "Extended Card Auth Test Agent");
}

#[tokio::test]
async fn rest_response_carries_cache_control_and_etag() {
    let base_url = spawn_test_server(
        |card| {
            card.security_schemes
                .insert("bearer".to_string(), bearer_scheme());
        },
        |server| server.with_auth_verifier(Arc::new(BearerVerifier)),
    )
    .await;

    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{base_url}/extendedAgentCard"))
        .header("A2A-Version", rusty_a2a::PROTOCOL_VERSION)
        .bearer_auth(VALID_TOKEN)
        .send()
        .await
        .expect("GET /extendedAgentCard");
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
        etag.contains("1.0.0"),
        "expected the ETag to reflect the card's version, got {etag:?}"
    );

    let conditional = http
        .get(format!("{base_url}/extendedAgentCard"))
        .header("A2A-Version", rusty_a2a::PROTOCOL_VERSION)
        .bearer_auth(VALID_TOKEN)
        .header("If-None-Match", &etag)
        .send()
        .await
        .expect("GET /extendedAgentCard (conditional)");
    assert_eq!(conditional.status(), 304);
}
