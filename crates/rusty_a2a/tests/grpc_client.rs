//! End-to-end test for `client::GrpcClient`: spins up an `AgentServices`
//! serving JSON-RPC+REST (for Agent Card discovery) on one port and gRPC
//! on another, then drives it entirely through the gRPC binding, covering
//! the same lifecycle `tests/integration.rs` covers for the JSON-RPC
//! client.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use rusty_a2a::client::{ClientError, GrpcClient};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{
    AgentExecutor, AgentServer, AuthContext, AuthVerifier, Credentials, EventSink, RequestContext,
};
use rusty_a2a::types::{
    AgentCard, AgentInterface, Artifact, HttpAuthSecurityScheme, Message, Part, SecurityRequirement,
    SecurityScheme, SendMessageConfiguration, SendMessageResult, StreamResponse, TaskPushNotificationConfig,
    TaskState,
};

const EXTENDED_CARD_TOKEN: &str = "extended-card-secret";

/// Accepts exactly one bearer token, for
/// `get_extended_agent_card_round_trips` - spec Section 13.3 makes
/// `GetExtendedAgentCard` authenticated unconditionally, so exercising it
/// needs a real `AuthVerifier` even though every other test in this file
/// hits an unauthenticated agent.
struct ExtendedCardVerifier;

#[async_trait]
impl AuthVerifier for ExtendedCardVerifier {
    async fn verify(
        &self,
        _requirement: &SecurityRequirement,
        credentials: &Credentials,
    ) -> Result<AuthContext> {
        match credentials.0.get("bearer") {
            Some(token) if token == EXTENDED_CARD_TOKEN => Ok(AuthContext::new("test-user")),
            _ => Err(A2aError::Unauthenticated("invalid bearer token".to_string())),
        }
    }
}

/// Same coverage as `tests/integration.rs`'s `TestAgent`.
struct TestAgent;

#[async_trait]
impl AgentExecutor for TestAgent {
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        let text = ctx.message.text();

        if text.contains("clarify") {
            events.message(Message::agent_text("what did you mean by that?"));
            return Ok(());
        }

        events.status(TaskState::Working);

        if text.contains("fail") {
            events.status_with_message(TaskState::Failed, Some(Message::agent_text("simulated failure")));
            return Ok(());
        }

        if text.contains("wait") {
            ctx.cancellation.cancelled().await;
            events.status(TaskState::Canceled);
            return Ok(());
        }

        events.artifact(Artifact::new("result", vec![Part::text("42")]));
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

/// Returns the HTTP origin (JSON-RPC/REST + Agent Card discovery) and the
/// gRPC endpoint, on two independent ports sharing one `Engine`.
async fn spawn_test_server() -> String {
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();
    let http_base_url = format!("http://{http_addr}");

    let grpc_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    drop(grpc_listener);
    let grpc_url = format!("http://{grpc_addr}");

    let mut card = AgentCard::new(
        "gRPC Client Test Agent",
        "An A2A agent used for rusty_a2a's gRPC client tests.",
        "0.0.0",
        AgentInterface::grpc(grpc_url),
    )
    .with_interface(AgentInterface::json_rpc(http_base_url.clone()))
    .with_streaming(true)
    .with_push_notifications(true);
    card.capabilities.extended_agent_card = Some(true);
    card.security_schemes.insert(
        "bearer".to_string(),
        SecurityScheme::HttpAuth {
            http_auth_security_scheme: HttpAuthSecurityScheme {
                description: None,
                scheme: "Bearer".to_string(),
                bearer_format: None,
            },
        },
    );
    let extended_card = card.clone();

    let services = AgentServer::new(card, Arc::new(TestAgent))
        .with_extended_card(extended_card)
        .with_auth_verifier(Arc::new(ExtendedCardVerifier))
        .build();

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

#[tokio::test]
async fn full_task_lifecycle_via_blocking_send() {
    let base_url = spawn_test_server().await;
    let (client, card) = GrpcClient::discover(&base_url).await.expect("discover");
    assert_eq!(card.name, "gRPC Client Test Agent");

    let result = client
        .send_message(Message::user_text("please compute"), None)
        .await
        .expect("send_message");

    let task = match result {
        SendMessageResult::Task { task } => task,
        SendMessageResult::Message { .. } => panic!("expected a task"),
    };
    assert_eq!(task.status.state, TaskState::Completed);
    assert_eq!(task.artifacts.len(), 1);
    assert_eq!(task.artifacts[0].parts[0].as_text(), Some("42"));

    let fetched = client.get_task(&task.id, None).await.expect("get_task");
    assert_eq!(fetched.status.state, TaskState::Completed);
    assert_eq!(fetched.id, task.id);
}

#[tokio::test]
async fn message_only_reply_creates_no_task() {
    let base_url = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("please clarify this"), None)
        .await
        .expect("send_message");

    match result {
        SendMessageResult::Message { message } => {
            assert_eq!(message.text(), "what did you mean by that?");
        }
        SendMessageResult::Task { .. } => panic!("expected a bare message, not a task"),
    }
}

#[tokio::test]
async fn list_tasks_finds_the_created_task() {
    let base_url = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("please compute"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let listed = client.list_tasks(Default::default()).await.expect("list_tasks");
    assert!(listed.tasks.iter().any(|t| t.id == task_id));
}

#[tokio::test]
async fn streaming_message_yields_ordered_events_ending_in_terminal_status() {
    let base_url = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");

    let mut stream = client
        .send_streaming_message(Message::user_text("please compute"), None)
        .await
        .expect("send_streaming_message");

    // Spec Section 3.1.2: since this turn is task-shaped, the stream MUST
    // begin with the `Task` object itself.
    let first = stream
        .next()
        .await
        .expect("first stream event")
        .expect("stream event");
    match first {
        StreamResponse::Task { task } => assert_eq!(task.status.state, TaskState::Submitted),
        other => panic!("expected the stream to lead with a Task, got {other:?}"),
    }

    let mut saw_working = false;
    let mut saw_artifact = false;
    let mut saw_completed = false;
    while let Some(event) = stream.next().await {
        match event.expect("stream event") {
            StreamResponse::StatusUpdate { status_update } => match status_update.status.state {
                TaskState::Working => saw_working = true,
                TaskState::Completed => saw_completed = true,
                other => panic!("unexpected state {other:?}"),
            },
            StreamResponse::ArtifactUpdate { .. } => saw_artifact = true,
            other => panic!("unexpected stream item: {other:?}"),
        }
    }
    assert!(saw_working && saw_artifact && saw_completed);
}

#[tokio::test]
async fn cancel_task_stops_a_waiting_executor() {
    let base_url = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");

    let config = SendMessageConfiguration {
        return_immediately: true,
        ..Default::default()
    };
    let result = client
        .send_message(Message::user_text("please wait forever"), Some(config))
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let canceled = client.cancel_task(&task_id).await.expect("cancel_task");
    assert_eq!(canceled.status.state, TaskState::Canceled);

    let err = client.cancel_task(&task_id).await.unwrap_err();
    match err {
        ClientError::Protocol(A2aError::TaskNotCancelable(_)) => {}
        other => panic!("expected TaskNotCancelable, got {other:?}"),
    }
}

#[tokio::test]
async fn subscribe_to_task_streams_updates_to_completion() {
    let base_url = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");

    let config = SendMessageConfiguration {
        return_immediately: true,
        ..Default::default()
    };
    let result = client
        .send_message(Message::user_text("please wait forever"), Some(config))
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    // Subscribing before the task reaches a terminal state (`SubscribeToTask`
    // rejects an already-terminal task on every binding, spec Section
    // 3.1.6) replays the buffered log so far, then continues live.
    let mut stream = client
        .subscribe_to_task(&task_id)
        .await
        .expect("subscribe_to_task");

    tokio::time::sleep(Duration::from_millis(20)).await;
    let cancel_client = client.clone();
    tokio::spawn(async move {
        let _ = cancel_client.cancel_task(&task_id).await;
    });

    let mut saw_canceled = false;
    while let Some(event) = stream.next().await {
        if let StreamResponse::StatusUpdate { status_update } = event.expect("stream event") {
            if status_update.status.state == TaskState::Canceled {
                saw_canceled = true;
                break;
            }
        }
    }
    assert!(saw_canceled);
}

#[tokio::test]
async fn task_not_found_maps_to_task_not_found_error() {
    let base_url = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");

    let err = client.get_task("does-not-exist", None).await.unwrap_err();
    match err {
        ClientError::Protocol(A2aError::TaskNotFound(_)) => {}
        other => panic!("expected TaskNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn push_notification_config_crud() {
    let base_url = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("please compute"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let mut config = TaskPushNotificationConfig::new("https://example.com/webhook");
    config.task_id = Some(task_id.clone());
    let created = client
        .create_push_notification_config(config)
        .await
        .expect("create_push_notification_config");
    let config_id = created.id.clone().expect("server-assigned id");

    let fetched = client
        .get_push_notification_config(&task_id, &config_id)
        .await
        .expect("get_push_notification_config");
    assert_eq!(fetched.url, "https://example.com/webhook");

    let listed = client
        .list_push_notification_configs(&task_id)
        .await
        .expect("list_push_notification_configs");
    assert_eq!(listed.configs.len(), 1);

    client
        .delete_push_notification_config(&task_id, &config_id)
        .await
        .expect("delete_push_notification_config");

    let err = client
        .get_push_notification_config(&task_id, &config_id)
        .await
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::TaskNotFound(_)) => {}
        other => panic!("expected TaskNotFound after delete, got {other:?}"),
    }
}

#[tokio::test]
async fn get_extended_agent_card_round_trips() {
    let base_url = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");
    let client = client.with_bearer_token(EXTENDED_CARD_TOKEN);

    let card = client
        .get_extended_agent_card()
        .await
        .expect("get_extended_agent_card");
    assert_eq!(card.name, "gRPC Client Test Agent");
}

#[tokio::test]
async fn grpc_and_jsonrpc_clients_share_the_same_task_store() {
    let base_url = spawn_test_server().await;
    let (grpc_client, _) = GrpcClient::discover(&base_url).await.expect("discover");
    let (jsonrpc_client, _) = rusty_a2a::client::A2aClient::discover(&base_url)
        .await
        .expect("discover");

    let result = grpc_client
        .send_message(Message::user_text("please compute"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let fetched = jsonrpc_client
        .get_task(&task_id, None)
        .await
        .expect("get_task via JSON-RPC");
    assert_eq!(fetched.id, task_id);
}
