//! What the gRPC binding does with a request's metadata and arguments, driven by a real `tonic`
//! client (the oracle in `common/tonic_oracle.rs`): the protocol version, credentials declared in a
//! mixed-case header, and `historyLength`. The REST and JSON-RPC bindings have their own tests of
//! the same rules; these hold the gRPC binding to them.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::tonic_oracle as pb;
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{
    AgentExecutor, AgentServer, AuthContext, AuthVerifier, Credentials, EventSink, RequestContext,
};
use rusty_a2a::types::{
    AgentCard, AgentInterface, ApiKeySecurityScheme, Message, SecurityRequirement, SecurityScheme,
    StringList, TaskState,
};
use tonic::transport::Channel;
use tonic::{Code, Request};

/// Completes every task with two history entries: the user's message and the agent's reply.
struct Replies;

#[async_trait]
impl AgentExecutor for Replies {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

const KEY: &str = "s3cret";

/// Accepts one API key, as the scheme named `key`.
struct KeyVerifier;

#[async_trait]
impl AuthVerifier for KeyVerifier {
    async fn verify(
        &self,
        _requirement: &SecurityRequirement,
        credentials: &Credentials,
    ) -> Result<AuthContext> {
        match credentials.0.get("key") {
            Some(key) if key == KEY => Ok(AuthContext::new("someone")),
            _ => Err(A2aError::Unauthenticated("bad key".to_string())),
        }
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// An agent with no security.
async fn start_open() -> String {
    serve(|port| {
        let card = AgentCard::new(
            "gRPC semantics test agent",
            "Replies.",
            "0.0.0",
            AgentInterface::json_rpc(format!("http://127.0.0.1:{port}")),
        );
        AgentServer::new(card, Arc::new(Replies))
    })
    .await
}

/// An agent whose API key travels in the header `X-Api-Key`, mixed case on purpose: gRPC
/// metadata keys are lowercase, so the binding has to fold the declared name.
async fn start_with_api_key() -> String {
    serve(|port| {
        let mut card = AgentCard::new(
            "gRPC semantics test agent",
            "Needs a key.",
            "0.0.0",
            AgentInterface::json_rpc(format!("http://127.0.0.1:{port}")),
        );
        card.security_schemes.insert(
            "key".to_string(),
            SecurityScheme::ApiKey {
                api_key_security_scheme: ApiKeySecurityScheme {
                    description: None,
                    location: "header".to_string(),
                    name: "X-Api-Key".to_string(),
                },
            },
        );
        // A scheme is only enforced once a requirement names it.
        card.security_requirements = vec![SecurityRequirement {
            schemes: HashMap::from([("key".to_string(), StringList { list: Vec::new() })]),
        }];
        AgentServer::new(card, Arc::new(Replies)).with_auth_verifier(Arc::new(KeyVerifier))
    })
    .await
}

/// Serves the agent `make` builds for a free port, and returns its URL. A port taken between
/// being picked and being bound (other tests are picking too) is tried again with another.
async fn serve(make: impl Fn(u16) -> AgentServer) -> String {
    for _ in 0..20 {
        let port = free_port();
        let services = make(port).build();
        let task = tokio::spawn(async move { services.serve_grpc(([127, 0, 0, 1], port)).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        if !task.is_finished() {
            return format!("http://127.0.0.1:{port}");
        }
    }
    panic!("no free port to serve on");
}

async fn connect(url: String) -> pb::a2a_service_client::A2aServiceClient<Channel> {
    let channel = Channel::from_shared(url).unwrap().connect().await.unwrap();
    pb::a2a_service_client::A2aServiceClient::new(channel)
}

fn with<T>(message: T, metadata: &[(&'static str, &str)]) -> Request<T> {
    let mut request = Request::new(message);
    for (key, value) in metadata {
        request.metadata_mut().insert(*key, value.parse().unwrap());
    }
    request
}

/// `message` as a request carrying the protocol version the binding speaks.
fn v<T>(message: T) -> Request<T> {
    with(message, &[("a2a-version", "1.0")])
}

fn hello() -> pb::SendMessageRequest {
    pb::SendMessageRequest {
        message: Some(pb::Message {
            message_id: "m1".to_string(),
            role: pb::Role::User as i32,
            parts: vec![pb::Part {
                content: Some(pb::part::Content::Text("hi".to_string())),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn an_unsupported_protocol_version_is_refused_with_its_reason() {
    let mut client = connect(start_open().await).await;
    let status = client
        .send_message(with(hello(), &[("a2a-version", "0.9")]))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(status.message().contains("0.9"), "{status:?}");
}

#[tokio::test]
async fn a_request_with_no_protocol_version_is_refused() {
    // The binding treats a missing version as the old one, which it does not speak.
    let mut client = connect(start_open().await).await;
    let status = client.send_message(with(hello(), &[])).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
}

#[tokio::test]
async fn a_key_declared_in_a_mixed_case_header_is_read_from_the_lowercase_metadata() {
    let mut client = connect(start_with_api_key().await).await;
    let missing = client
        .send_message(with(hello(), &[("a2a-version", "1.0")]))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::Unauthenticated);
    let accepted = client
        .send_message(with(hello(), &[("a2a-version", "1.0"), ("x-api-key", KEY)]))
        .await;
    assert!(accepted.is_ok(), "{accepted:?}");
    let wrong = client
        .send_message(with(hello(), &[("a2a-version", "1.0"), ("x-api-key", "nope")]))
        .await
        .unwrap_err();
    assert_eq!(wrong.code(), Code::Unauthenticated);
}

#[tokio::test]
async fn get_task_honours_history_length() {
    let mut client = connect(start_open().await).await;
    let sent = client.send_message(v(hello())).await.unwrap().into_inner();
    let id = match sent.payload {
        Some(pb::send_message_response::Payload::Task(task)) => task.id,
        other => panic!("expected a task, got {other:?}"),
    };
    let history = |length: Option<i32>| {
        let mut client = client.clone();
        let id = id.clone();
        async move {
            client
                .get_task(v(pb::GetTaskRequest {
                    id,
                    history_length: length,
                    ..Default::default()
                }))
                .await
                .unwrap()
                .into_inner()
                .history
                .len()
        }
    };
    assert_eq!(history(None).await, 2);
    assert_eq!(history(Some(1)).await, 1);
    assert_eq!(history(Some(0)).await, 0);
}
