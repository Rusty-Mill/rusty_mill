//! End-to-end coverage for the two enforcement gaps closed alongside the
//! JSON-RPC/REST/gRPC bindings: `AgentCard.securitySchemes`/
//! `securityRequirements` enforcement (via a pluggable `AuthVerifier`),
//! and actual webhook delivery of push notifications (rather than just
//! CRUD storage of the config).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{A2aClient, ClientError};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{
    AgentExecutor, AgentServer, AuthContext, AuthVerifier, Credentials, EventSink, RequestContext,
};
use rusty_a2a::types::{
    AgentCard, AgentInterface, HttpAuthSecurityScheme, Message, SecurityRequirement, SecurityScheme,
    StringList, TaskPushNotificationConfig, TaskState,
};

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(
            TaskState::Completed,
            Some(Message::agent_text(format!("you said: {}", ctx.message.text()))),
        );
        Ok(())
    }
}

const VALID_TOKEN: &str = "secret-token";

/// Accepts exactly one bearer token; rejects everything else, including a
/// missing one.
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

fn bearer_scheme() -> SecurityScheme {
    SecurityScheme::HttpAuth {
        http_auth_security_scheme: HttpAuthSecurityScheme {
            description: None,
            scheme: "Bearer".to_string(),
            bearer_format: None,
        },
    }
}

fn bearer_requirement() -> SecurityRequirement {
    SecurityRequirement {
        schemes: HashMap::from([("bearer".to_string(), StringList { list: Vec::new() })]),
    }
}

/// A server whose `AgentCard` requires the `bearer` scheme for every
/// operation, backed by a real [`BearerVerifier`].
async fn spawn_secured_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut card = AgentCard::new(
        "Secured Agent",
        "An A2A agent requiring bearer auth, for rusty_a2a's security tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    card.security_schemes
        .insert("bearer".to_string(), bearer_scheme());
    card.security_requirements = vec![bearer_requirement()];

    let server = AgentServer::new(card, Arc::new(EchoAgent)).with_auth_verifier(Arc::new(BearerVerifier));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

fn expect_unauthenticated(err: ClientError) {
    match err {
        ClientError::Protocol(A2aError::Unauthenticated(_)) => {}
        other => panic!("expected Unauthenticated, got {other:?}"),
    }
}

#[tokio::test]
async fn unauthenticated_json_rpc_request_is_rejected() {
    let base_url = spawn_secured_server().await;
    let client = A2aClient::new(format!("{base_url}/"));

    let err = client
        .send_message(Message::user_text("hi"), None)
        .await
        .unwrap_err();
    expect_unauthenticated(err);
}

#[tokio::test]
async fn wrong_bearer_token_is_rejected() {
    let base_url = spawn_secured_server().await;
    let client = A2aClient::new(format!("{base_url}/")).with_bearer_token("not-the-right-token");

    let err = client
        .send_message(Message::user_text("hi"), None)
        .await
        .unwrap_err();
    expect_unauthenticated(err);
}

#[tokio::test]
async fn correct_bearer_token_is_accepted() {
    let base_url = spawn_secured_server().await;
    let client = A2aClient::new(format!("{base_url}/")).with_bearer_token(VALID_TOKEN);

    let result = client
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message should succeed with valid credentials");
    let task = result.as_task().expect("expected a task");
    assert_eq!(task.status.state, TaskState::Completed);
}

#[tokio::test]
async fn rest_binding_enforces_the_same_security_requirements() {
    let base_url = spawn_secured_server().await;
    let http = reqwest::Client::new();

    let unauthenticated = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /message:send");
    assert_eq!(unauthenticated.status(), 401);

    let authenticated = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .bearer_auth(VALID_TOKEN)
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /message:send");
    assert_eq!(authenticated.status(), 200);
}

/// Declaring `securityRequirements` without registering an `AuthVerifier`
/// must fail closed - silently letting every request through would defeat
/// the point of declaring requirements in the first place.
#[tokio::test]
async fn security_requirements_without_a_verifier_fail_closed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut card = AgentCard::new(
        "Misconfigured Agent",
        "Declares securityRequirements but never registers a verifier.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    card.security_schemes
        .insert("bearer".to_string(), bearer_scheme());
    card.security_requirements = vec![bearer_requirement()];

    let server = AgentServer::new(card, Arc::new(EchoAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let client = A2aClient::new(format!("{base_url}/")).with_bearer_token(VALID_TOKEN);
    let err = client
        .send_message(Message::user_text("hi"), None)
        .await
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::Internal(_)) => {}
        other => panic!("expected Internal (fail-closed misconfiguration error), got {other:?}"),
    }
}

/// `GetExtendedAgentCard` is authenticated even though the base card
/// declares no `securityRequirements` of its own, as long as a verifier
/// is configured and at least one `securityScheme` is declared to check
/// against.
#[tokio::test]
async fn extended_agent_card_requires_auth_when_a_verifier_is_configured() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut card = AgentCard::new(
        "Extended Card Agent",
        "Declares an extended card and a bearer scheme, but no base-level securityRequirements.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    card.capabilities.extended_agent_card = Some(true);
    card.security_schemes
        .insert("bearer".to_string(), bearer_scheme());
    // Deliberately no `card.security_requirements` - the extended card
    // check must fall back to the declared schemes.

    let extended = AgentCard::new(
        "Extended Card Agent (extended)",
        "The extended card, only visible to authenticated callers.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );

    let server = AgentServer::new(card, Arc::new(EchoAgent))
        .with_extended_card(extended)
        .with_auth_verifier(Arc::new(BearerVerifier));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let unauthenticated_client = A2aClient::new(format!("{base_url}/"));
    let err = unauthenticated_client
        .get_extended_agent_card()
        .await
        .unwrap_err();
    expect_unauthenticated(err);

    let authenticated_client = A2aClient::new(format!("{base_url}/")).with_bearer_token(VALID_TOKEN);
    let card = authenticated_client
        .get_extended_agent_card()
        .await
        .expect("should succeed with valid credentials");
    assert_eq!(card.name, "Extended Card Agent (extended)");
}

// --- Push notification delivery (spec Section 4.3) ---

type WebhookDelivery = (Option<String>, serde_json::Value);

/// A minimal webhook receiver: records every POSTed JSON body (and its
/// `X-A2A-Notification-Token` header) it gets.
#[derive(Clone, Default)]
struct WebhookReceiver {
    received: Arc<Mutex<Vec<WebhookDelivery>>>,
}

async fn spawn_webhook_receiver() -> (String, WebhookReceiver) {
    use axum::extract::State as AxumState;
    use axum::routing::post;

    async fn handler(
        AxumState(receiver): AxumState<WebhookReceiver>,
        headers: axum::http::HeaderMap,
        axum::Json(body): axum::Json<serde_json::Value>,
    ) -> axum::http::StatusCode {
        let token = headers
            .get("X-A2A-Notification-Token")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        receiver.received.lock().unwrap().push((token, body));
        axum::http::StatusCode::OK
    }

    let receiver = WebhookReceiver::default();
    let app = axum::Router::new()
        .route("/webhook", post(handler))
        .with_state(receiver.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (format!("http://{addr}/webhook"), receiver)
}

/// Goes `Working` -> (a brief pause, so the test has a window to register
/// a push notification config while the task is still in flight) ->
/// `Completed`.
struct SlowAgent;

#[async_trait]
impl AgentExecutor for SlowAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        tokio::time::sleep(Duration::from_millis(150)).await;
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_push_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "Push Test Agent",
        "An A2A agent used for rusty_a2a's push notification delivery tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    )
    .with_push_notifications(true);

    let server = AgentServer::new(card, Arc::new(SlowAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[tokio::test]
async fn completed_task_delivers_a_push_notification_to_the_configured_webhook() {
    let base_url = spawn_push_test_server().await;
    let (webhook_url, receiver) = spawn_webhook_receiver().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    // Non-blocking send: returns as soon as the task is `Working`, well
    // before `SlowAgent`'s 150ms pause completes it.
    let config = rusty_a2a::types::SendMessageConfiguration {
        return_immediately: true,
        ..Default::default()
    };
    let started = client
        .send_message(Message::user_text("hello"), Some(config))
        .await
        .expect("send_message");
    let task_id = started.as_task().expect("expected a task").id.clone();
    assert!(!started.as_task().unwrap().status.state.is_terminal());

    let mut push_config = TaskPushNotificationConfig::new(webhook_url);
    push_config.task_id = Some(task_id.clone());
    push_config.token = Some("correlation-token".to_string());
    client
        .create_push_notification_config(push_config)
        .await
        .expect("create_push_notification_config");

    // Poll the webhook receiver for the `Completed` delivery, fired once
    // `SlowAgent` finishes.
    let mut delivered = Vec::new();
    for _ in 0..50 {
        let got = receiver.received.lock().unwrap().clone();
        if got
            .iter()
            .any(|(_, body)| body["status"]["state"] == "TASK_STATE_COMPLETED")
        {
            delivered = got;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let (token, body) = delivered
        .iter()
        .find(|(_, body)| body["status"]["state"] == "TASK_STATE_COMPLETED")
        .expect("expected a push notification delivery for the completed task");
    assert_eq!(token.as_deref(), Some("correlation-token"));
    assert_eq!(body["id"], task_id);
}

/// `SendMessageConfiguration.taskPushNotificationConfig` (spec Section
/// 3.1.1) lets a client register push-notification delivery in the same
/// request that creates the task, since it doesn't yet know the
/// server-assigned task id to make a separate
/// `CreateTaskPushNotificationConfig` call. Unlike the test above, no
/// separate registration call happens at all here.
#[tokio::test]
async fn send_message_configuration_registers_push_notifications_at_task_creation() {
    let base_url = spawn_push_test_server().await;
    let (webhook_url, receiver) = spawn_webhook_receiver().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let mut push_config = TaskPushNotificationConfig::new(webhook_url);
    push_config.token = Some("inline-token".to_string());
    let config = rusty_a2a::types::SendMessageConfiguration {
        return_immediately: true,
        task_push_notification_config: Some(push_config),
        ..Default::default()
    };
    let started = client
        .send_message(Message::user_text("hello"), Some(config))
        .await
        .expect("send_message");
    let task_id = started.as_task().expect("expected a task").id.clone();
    assert!(!started.as_task().unwrap().status.state.is_terminal());

    let mut delivered = Vec::new();
    for _ in 0..50 {
        let got = receiver.received.lock().unwrap().clone();
        if got
            .iter()
            .any(|(_, body)| body["status"]["state"] == "TASK_STATE_COMPLETED")
        {
            delivered = got;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let (token, body) = delivered
        .iter()
        .find(|(_, body)| body["status"]["state"] == "TASK_STATE_COMPLETED")
        .expect("expected a push notification delivery for the completed task");
    assert_eq!(token.as_deref(), Some("inline-token"));
    assert_eq!(body["id"], task_id);

    // Also reachable via the ordinary CRUD read path, with the
    // server-assigned `taskId`/`id` filled in.
    let listed = client
        .list_push_notification_configs(&task_id)
        .await
        .expect("list_push_notification_configs");
    assert_eq!(listed.configs.len(), 1);
    assert_eq!(listed.configs[0].task_id.as_deref(), Some(task_id.as_str()));
}

/// A continuation turn (the client resumes an existing task by sending a
/// new message with `taskId` set) must NOT re-register a fresh duplicate
/// push config even if it happens to echo the same
/// `taskPushNotificationConfig` again - the client-supplied config has no
/// server-assigned `id`, so registering it a second time would silently
/// double every future delivery.
#[tokio::test]
async fn continuation_turns_do_not_duplicate_the_inline_push_config() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    struct AskThenCompleteAgent;
    #[async_trait]
    impl AgentExecutor for AskThenCompleteAgent {
        async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
            events.status(TaskState::Working);
            if ctx.task.is_none() {
                events.status_with_message(
                    TaskState::InputRequired,
                    Some(Message::agent_text("more info please")),
                );
            } else {
                events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
            }
            Ok(())
        }
    }

    let card = AgentCard::new(
        "Ask Then Complete Test Agent",
        "An A2A agent used for rusty_a2a's inline-push-config-not-duplicated test.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    )
    .with_push_notifications(true);
    let server = AgentServer::new(card, Arc::new(AskThenCompleteAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let mut push_config = TaskPushNotificationConfig::new("https://example.com/webhook");
    push_config.token = Some("repeated-token".to_string());
    let config = rusty_a2a::types::SendMessageConfiguration {
        task_push_notification_config: Some(push_config.clone()),
        ..Default::default()
    };
    let first = client
        .send_message(Message::user_text("hi"), Some(config))
        .await
        .expect("send_message");
    let task_id = first.as_task().expect("expected a task").id.clone();
    assert_eq!(first.as_task().unwrap().status.state, TaskState::InputRequired);

    // Continuation turn, echoing the very same inline config again.
    let mut follow_up = Message::user_text("here's more info");
    follow_up.task_id = Some(task_id.clone());
    let config2 = rusty_a2a::types::SendMessageConfiguration {
        task_push_notification_config: Some(push_config),
        ..Default::default()
    };
    let second = client
        .send_message(follow_up, Some(config2))
        .await
        .expect("send_message (continuation)");
    assert_eq!(second.as_task().unwrap().status.state, TaskState::Completed);

    let listed = client
        .list_push_notification_configs(&task_id)
        .await
        .expect("list_push_notification_configs");
    assert_eq!(
        listed.configs.len(),
        1,
        "expected exactly one config, not a duplicate per turn: {:?}",
        listed.configs
    );
}
