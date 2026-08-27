//! End-to-end test: spins up a real `AgentServer` on a local TCP port and
//! exercises it with a real `A2aClient`, covering the task lifecycle,
//! streaming, cancellation, and push notification config CRUD.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use rusty_a2a::client::A2aClient;
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{
    AgentCard, AgentInterface, Artifact, Message, Part, SendMessageConfiguration, SendMessageResult,
    StreamResponse, TaskPushNotificationConfig, TaskState,
};

/// An executor covering every code path the harness supports:
/// - a message containing "clarify" gets a bare `Message` reply, no task.
/// - a message containing "fail" produces a task that ends `Failed`.
/// - a message containing "wait" moves to `Working` and then blocks on
///   `ctx.cancellation`, to exercise `CancelTask`.
/// - anything else produces a task that goes `Working` -> emits one
///   artifact -> `Completed`.
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

    let card = AgentCard::new(
        "Test Agent",
        "An A2A agent used for rusty_a2a's integration tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    )
    .with_streaming(true)
    .with_push_notifications(true);

    let server = AgentServer::new(card, Arc::new(TestAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });

    // Give the listener a moment to start accepting.
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[tokio::test]
async fn full_task_lifecycle_via_blocking_send() {
    let base_url = spawn_test_server().await;
    let (client, card) = A2aClient::discover(&base_url).await.expect("discover");
    assert_eq!(card.name, "Test Agent");
    assert_eq!(card.capabilities.streaming, Some(true));

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

    // GetTask should reflect the same final state.
    let fetched = client.get_task(&task.id, None).await.expect("get_task");
    assert_eq!(fetched.status.state, TaskState::Completed);
    assert_eq!(fetched.id, task.id);
}

#[tokio::test]
async fn message_only_reply_creates_no_task() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

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
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("please fail this"), None)
        .await
        .expect("send_message");

    let task = result.as_task().expect("expected a task").clone();
    assert_eq!(task.status.state, TaskState::Failed);
}

#[tokio::test]
async fn non_blocking_send_returns_before_completion() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let config = SendMessageConfiguration {
        return_immediately: true,
        ..Default::default()
    };
    let result = client
        .send_message(Message::user_text("please compute"), Some(config))
        .await
        .expect("send_message");

    let task = result.as_task().expect("expected a task");
    assert!(
        !task.status.state.is_terminal(),
        "task should not be terminal yet"
    );

    // Poll until it completes.
    let task_id = task.id.clone();
    let mut final_state = task.status.state;
    for _ in 0..50 {
        let t = client.get_task(&task_id, None).await.expect("get_task");
        final_state = t.status.state;
        if final_state.is_terminal() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(final_state, TaskState::Completed);
}

#[tokio::test]
async fn streaming_message_yields_ordered_events_ending_in_terminal_status() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

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
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let config = SendMessageConfiguration {
        return_immediately: true,
        ..Default::default()
    };
    let result = client
        .send_message(Message::user_text("please wait forever"), Some(config))
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    // Give the executor a moment to reach its `wait` point.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let canceled = client.cancel_task(&task_id).await.expect("cancel_task");
    assert_eq!(canceled.status.state, TaskState::Canceled);

    // A second cancel on an already-terminal task must fail.
    let err = client.cancel_task(&task_id).await.unwrap_err();
    match err {
        rusty_a2a::client::ClientError::Protocol(A2aError::TaskNotCancelable(_)) => {}
        other => panic!("expected TaskNotCancelable, got {other:?}"),
    }
}

#[tokio::test]
async fn subscribe_to_terminal_task_is_rejected() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("please compute"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    // A terminal task fails before the SSE stream even opens (spec Section
    // 3.1.6), so the error surfaces directly from `subscribe_to_task`
    // rather than as the stream's first item.
    match client.subscribe_to_task(&task_id).await {
        Err(rusty_a2a::client::ClientError::Protocol(A2aError::UnsupportedOperation(_))) => {}
        Err(other) => panic!("expected UnsupportedOperationError, got {other:?}"),
        Ok(_) => panic!("expected UnsupportedOperationError, got a stream"),
    }
}

#[tokio::test]
async fn push_notification_config_crud() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

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
        rusty_a2a::client::ClientError::Protocol(A2aError::TaskNotFound(_)) => {}
        other => panic!("expected TaskNotFound after delete, got {other:?}"),
    }
}

#[tokio::test]
async fn version_header_mismatch_is_rejected() {
    let base_url = spawn_test_server().await;
    let client = A2aClient::new(format!("{base_url}/")).with_protocol_version("0.3");

    let err = client
        .send_message(Message::user_text("hi"), None)
        .await
        .unwrap_err();
    match err {
        rusty_a2a::client::ClientError::Protocol(A2aError::VersionNotSupported(v)) => assert_eq!(v, "0.3"),
        other => panic!("expected VersionNotSupported, got {other:?}"),
    }
}

// --- HTTP+JSON/REST binding (spec Section 11) ---
//
// `AgentServer::into_router` mounts the REST binding on the same port as
// JSON-RPC, so `spawn_test_server`'s server already serves it; these tests
// talk to it directly with a bare `reqwest::Client` rather than
// `A2aClient` (which only speaks JSON-RPC).

#[tokio::test]
async fn rest_send_message_and_get_task_round_trip() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let send_resp = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "please compute"}]}
        }))
        .send()
        .await
        .expect("POST /message:send");

    assert_eq!(send_resp.status(), 200);
    assert_eq!(
        send_resp.headers().get("content-type").unwrap(),
        "application/a2a+json"
    );
    let body: serde_json::Value = send_resp.json().await.expect("response body");
    let task = &body["task"];
    assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED");
    let task_id = task["id"].as_str().expect("task id").to_string();

    let get_resp = http
        .get(format!("{base_url}/tasks/{task_id}"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tasks/{id}");
    assert_eq!(get_resp.status(), 200);
    let fetched: serde_json::Value = get_resp.json().await.expect("response body");
    assert_eq!(fetched["id"], task_id);
    assert_eq!(fetched["status"]["state"], "TASK_STATE_COMPLETED");
    assert_eq!(fetched["artifacts"][0]["parts"][0]["text"], "42");
}

#[tokio::test]
async fn rest_streaming_message_yields_raw_stream_response_events() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .post(format!("{base_url}/message:stream"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "please compute"}]}
        }))
        .send()
        .await
        .expect("POST /message:stream");

    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers().get("content-type").unwrap(), "text/event-stream");

    use eventsource_stream::Eventsource;
    let mut events = resp.bytes_stream().eventsource();
    let mut saw_working = false;
    let mut saw_completed = false;
    while let Some(event) = events.next().await {
        let event = event.expect("sse event");
        // REST streaming carries the bare `StreamResponse` object, with no
        // `{"jsonrpc":"2.0", "id":..., "result": ...}` envelope around it.
        let value: serde_json::Value = serde_json::from_str(&event.data).expect("event json");
        assert!(
            value.get("jsonrpc").is_none(),
            "REST SSE must not be JSON-RPC-wrapped"
        );
        if let Some(state) = value["statusUpdate"]["status"]["state"].as_str() {
            match state {
                "TASK_STATE_WORKING" => saw_working = true,
                "TASK_STATE_COMPLETED" => saw_completed = true,
                other => panic!("unexpected state {other}"),
            }
        }
    }
    assert!(saw_working && saw_completed);
}

#[tokio::test]
async fn rest_error_uses_google_rpc_status_shape_and_real_http_code() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .get(format!("{base_url}/tasks/does-not-exist"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tasks/{id}");

    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.expect("response body");
    assert_eq!(body["error"]["code"], 404);
    assert_eq!(body["error"]["status"], "NOT_FOUND");
    assert_eq!(body["error"]["details"][0]["reason"], "TASK_NOT_FOUND");
    assert_eq!(body["error"]["details"][0]["domain"], "a2a-protocol.org");
}

#[tokio::test]
async fn rest_cancel_task_action_suffix_routing() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    // A task that blocks in `Working` until canceled (see `TestAgent`).
    let send_resp: serde_json::Value = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "please wait forever"}]},
            "configuration": {"returnImmediately": true}
        }))
        .send()
        .await
        .expect("POST /message:send")
        .json()
        .await
        .expect("response body");
    let task_id = send_resp["task"]["id"].as_str().expect("task id").to_string();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // `/tasks/{id}:cancel` - the colon-suffixed path that motivated this
    // module's routing workaround.
    let cancel_resp = http
        .post(format!("{base_url}/tasks/{task_id}:cancel"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("POST /tasks/{id}:cancel");
    assert_eq!(cancel_resp.status(), 200);
    let canceled: serde_json::Value = cancel_resp.json().await.expect("response body");
    assert_eq!(canceled["status"]["state"], "TASK_STATE_CANCELED");
}

#[tokio::test]
async fn rest_push_notification_config_crud() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let send_resp: serde_json::Value = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "please compute"}]}
        }))
        .send()
        .await
        .expect("POST /message:send")
        .json()
        .await
        .expect("response body");
    let task_id = send_resp["task"]["id"].as_str().expect("task id").to_string();

    let created: serde_json::Value = http
        .post(format!("{base_url}/tasks/{task_id}/pushNotificationConfigs"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({"url": "https://example.com/webhook"}))
        .send()
        .await
        .expect("POST .../pushNotificationConfigs")
        .json()
        .await
        .expect("response body");
    let config_id = created["id"].as_str().expect("config id").to_string();
    assert_eq!(created["url"], "https://example.com/webhook");

    let listed: serde_json::Value = http
        .get(format!("{base_url}/tasks/{task_id}/pushNotificationConfigs"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET .../pushNotificationConfigs")
        .json()
        .await
        .expect("response body");
    assert_eq!(listed["configs"].as_array().unwrap().len(), 1);

    let delete_resp = http
        .delete(format!(
            "{base_url}/tasks/{task_id}/pushNotificationConfigs/{config_id}"
        ))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("DELETE .../pushNotificationConfigs/{configId}");
    assert_eq!(delete_resp.status(), 204);

    let get_after_delete = http
        .get(format!(
            "{base_url}/tasks/{task_id}/pushNotificationConfigs/{config_id}"
        ))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET .../pushNotificationConfigs/{configId}");
    assert_eq!(get_after_delete.status(), 404);
}

#[tokio::test]
async fn rest_and_jsonrpc_bindings_share_the_same_task_store() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");
    let http = reqwest::Client::new();

    // Create via JSON-RPC (through `A2aClient`)...
    let result = client
        .send_message(Message::user_text("please compute"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    // ...fetch the same task via REST.
    let resp = http
        .get(format!("{base_url}/tasks/{task_id}"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tasks/{id}");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body");
    assert_eq!(body["id"], task_id);
}
