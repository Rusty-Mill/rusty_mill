//! End-to-end test for `client::RestClient`: spins up a real `AgentServer`
//! (which serves JSON-RPC and REST on the same port) and drives it
//! entirely through the REST binding, covering the same lifecycle
//! `tests/integration.rs` covers for the JSON-RPC client.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use rusty_a2a::client::{ClientError, RestClient};
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

/// Same coverage as `tests/integration.rs`'s `TestAgent`: "clarify" ->
/// bare message, "fail" -> a `Failed` task, "wait" -> blocks on
/// cancellation, anything else -> one artifact then `Completed`.
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

async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut card = AgentCard::new(
        "REST Client Test Agent",
        "An A2A agent used for rusty_a2a's REST client tests.",
        "0.0.0",
        AgentInterface::http_json(base_url.clone()),
    )
    .with_interface(AgentInterface::json_rpc(base_url.clone()))
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

    let server = AgentServer::new(card, Arc::new(TestAgent))
        .with_extended_card(extended_card)
        .with_auth_verifier(Arc::new(ExtendedCardVerifier));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[tokio::test]
async fn full_task_lifecycle_via_blocking_send() {
    let base_url = spawn_test_server().await;
    let (client, card) = RestClient::discover(&base_url).await.expect("discover");
    assert_eq!(card.name, "REST Client Test Agent");

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
    let (client, _) = RestClient::discover(&base_url).await.expect("discover");

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
async fn failed_task_reports_failed_state() {
    let base_url = spawn_test_server().await;
    let (client, _) = RestClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("please fail this"), None)
        .await
        .expect("send_message");

    let task = result.as_task().expect("expected a task").clone();
    assert_eq!(task.status.state, TaskState::Failed);
}

#[tokio::test]
async fn list_tasks_finds_the_created_task() {
    let base_url = spawn_test_server().await;
    let (client, _) = RestClient::discover(&base_url).await.expect("discover");

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
    let (client, _) = RestClient::discover(&base_url).await.expect("discover");

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
    let (client, _) = RestClient::discover(&base_url).await.expect("discover");

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
async fn subscribe_to_task_streams_updates() {
    let base_url = spawn_test_server().await;
    let (client, _) = RestClient::discover(&base_url).await.expect("discover");

    let config = SendMessageConfiguration {
        return_immediately: true,
        ..Default::default()
    };
    let result = client
        .send_message(Message::user_text("please wait forever"), Some(config))
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let mut stream = client
        .subscribe_to_task(&task_id)
        .await
        .expect("subscribe_to_task");

    // Cancel concurrently so the subscription has a terminal event to
    // observe rather than hanging forever.
    tokio::time::sleep(Duration::from_millis(20)).await;
    let cancel_client = RestClient::new(base_url.clone());
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
    let (client, _) = RestClient::discover(&base_url).await.expect("discover");

    let err = client.get_task("does-not-exist", None).await.unwrap_err();
    match err {
        ClientError::Protocol(A2aError::TaskNotFound(_)) => {}
        other => panic!("expected TaskNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn push_notification_config_crud() {
    let base_url = spawn_test_server().await;
    let (client, _) = RestClient::discover(&base_url).await.expect("discover");

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
    let (client, _) = RestClient::discover(&base_url).await.expect("discover");
    let client = client.with_bearer_token(EXTENDED_CARD_TOKEN);

    let card = client
        .get_extended_agent_card()
        .await
        .expect("get_extended_agent_card");
    assert_eq!(card.name, "REST Client Test Agent");
}

#[tokio::test]
async fn rest_and_jsonrpc_clients_share_the_same_task_store() {
    let base_url = spawn_test_server().await;
    let (rest_client, _) = RestClient::discover(&base_url).await.expect("discover");
    let (jsonrpc_client, _) = rusty_a2a::client::A2aClient::discover(&base_url)
        .await
        .expect("discover");

    let result = rest_client
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

/// Pins the HTTP method, which the tests above cannot: this crate's own
/// server accepts both `GET` and `POST` on `/tasks/{id}:subscribe`, so a
/// passing subscribe proves only that one of them worked.
///
/// This server implements the spec-literal `GET` binding and nothing else —
/// which is all another SDK's server has any reason to implement — and
/// answers `405` to anything that arrives as a `POST`.
#[tokio::test]
async fn subscribe_uses_the_spec_literal_get_binding() {
    use axum::extract::Path;
    use axum::http::StatusCode;
    use axum::response::sse::{Event, Sse};
    use axum::response::IntoResponse;
    use axum::routing::get;

    async fn subscribe(Path(id_and_action): Path<String>) -> axum::response::Response {
        if id_and_action.rsplit_once(':').map(|(_, a)| a) != Some("subscribe") {
            return StatusCode::NOT_FOUND.into_response();
        }
        // One terminal status event, then end of stream.
        let event = Event::default().json_data(serde_json::json!({
            "statusUpdate": {
                "taskId": "t-1",
                "contextId": "c-1",
                "status": {"state": "TASK_STATE_COMPLETED"},
            }
        }));
        Sse::new(futures_util::stream::once(async move { event })).into_response()
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    // `get(...)` alone: axum answers 405 Method Not Allowed to a POST on a
    // route that declares no POST handler.
    let router = axum::Router::new().route("/tasks/:id", get(subscribe));
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let client = RestClient::new(&base_url);
    let mut stream = client
        .subscribe_to_task("t-1")
        .await
        .expect("a GET-only server must accept this subscription");

    let event = stream.next().await.expect("one event").expect("decodes");
    match event {
        StreamResponse::StatusUpdate { status_update } => {
            assert_eq!(status_update.status.state, TaskState::Completed);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
