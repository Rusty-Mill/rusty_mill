//! Covers `SubscribeToTask` reconnection replay (spec Section 3.1.6): a
//! caller that reconnects mid-stream should catch up on events it missed,
//! not just see a point-in-time snapshot of where things stand now.
//!
//! `SteppedAgent` lets each test control exactly when the agent advances
//! past each event, via `Notify`s shared in-process between the test and
//! the executor (no network involved in that signaling) - this makes the
//! "disconnect after event N, reconnect, expect events N+1.." scenarios
//! deterministic instead of timing-dependent.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Artifact, Message, Part, TaskState};
use tokio::sync::Notify;

/// Goes `Working` -> waits for `advance` to be notified -> emits an
/// artifact -> waits for `advance` again -> `Completed`.
struct SteppedAgent {
    advance: Arc<Notify>,
}

#[async_trait]
impl AgentExecutor for SteppedAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        self.advance.notified().await;
        events.artifact(Artifact::new("result", vec![Part::text("42")]));
        self.advance.notified().await;
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_stepped_server() -> (String, Arc<Notify>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let advance = Arc::new(Notify::new());
    let card = AgentCard::new(
        "Subscribe Replay Test Agent",
        "An A2A agent used for rusty_a2a's SubscribeToTask replay tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    )
    .with_streaming(true);

    let server = AgentServer::new(
        card,
        Arc::new(SteppedAgent {
            advance: advance.clone(),
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (base_url, advance)
}

/// Starts the task (non-blocking, so the response arrives while the task
/// is still `Working`) and returns its id.
async fn start_task(http: &reqwest::Client, base_url: &str) -> String {
    let resp: serde_json::Value = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]},
            "configuration": {"returnImmediately": true}
        }))
        .send()
        .await
        .expect("POST /message:send")
        .json()
        .await
        .expect("response body");
    resp["task"]["id"].as_str().expect("task id").to_string()
}

#[tokio::test]
async fn rest_subscribe_reconnect_replays_missed_events_via_last_event_id() {
    let (base_url, advance) = spawn_stepped_server().await;
    let http = reqwest::Client::new();
    let task_id = start_task(&http, &base_url).await;

    // First connection: a fresh subscribe (no Last-Event-ID) MUST begin
    // with a `Task` snapshot (spec Section 3.1.6), then the initial
    // `Working` event (already buffered by the time we subscribe, since
    // `start_task` waited for it); disconnect without advancing the agent
    // further.
    let first_resp = http
        .post(format!("{base_url}/tasks/{task_id}:subscribe"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("POST /tasks/{id}:subscribe");
    let mut first_events = first_resp.bytes_stream().eventsource();
    let lead = first_events.next().await.expect("lead event").expect("sse event");
    let lead_value: serde_json::Value = serde_json::from_str(&lead.data).unwrap();
    assert_eq!(lead_value["task"]["id"], task_id);
    let first = first_events
        .next()
        .await
        .expect("first event")
        .expect("sse event");
    let first_value: serde_json::Value = serde_json::from_str(&first.data).unwrap();
    assert_eq!(
        first_value["statusUpdate"]["status"]["state"],
        "TASK_STATE_WORKING"
    );
    let first_id: u64 = first.id.parse().expect("numeric SSE id");
    drop(first_events); // disconnect

    // Advance past the artifact update while nobody is subscribed; it
    // must still land in the task's event log.
    advance.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Reconnect with Last-Event-ID set to what connection #1 already saw.
    let second_resp = http
        .post(format!("{base_url}/tasks/{task_id}:subscribe"))
        .header("A2A-Version", "1.0")
        .header("Last-Event-ID", first_id.to_string())
        .send()
        .await
        .expect("POST /tasks/{id}:subscribe (reconnect)");
    let mut second_events = second_resp.bytes_stream().eventsource();

    // First thing on the reconnected stream must be the missed artifact
    // update, NOT a repeat of the `Working` status from connection #1.
    let replayed = second_events
        .next()
        .await
        .expect("replayed event")
        .expect("sse event");
    let replayed_id: u64 = replayed.id.parse().expect("numeric SSE id");
    assert!(
        replayed_id > first_id,
        "replay must not repeat an already-seen event"
    );
    let replayed_value: serde_json::Value = serde_json::from_str(&replayed.data).unwrap();
    assert!(
        replayed_value.get("artifactUpdate").is_some(),
        "expected the missed artifact update to be replayed first, got {replayed_value:?}"
    );

    // Now let the agent finish; the reconnected stream's live tail must
    // deliver the completion and then close.
    advance.notify_one();
    let completion = second_events
        .next()
        .await
        .expect("completion event")
        .expect("sse event");
    let completion_value: serde_json::Value = serde_json::from_str(&completion.data).unwrap();
    assert_eq!(
        completion_value["statusUpdate"]["status"]["state"],
        "TASK_STATE_COMPLETED"
    );
    assert!(
        second_events.next().await.is_none(),
        "stream should close after the terminal event"
    );
}

#[tokio::test]
async fn rest_get_subscribe_is_the_spec_literal_binding_and_still_works() {
    let (base_url, advance) = spawn_stepped_server().await;
    let http = reqwest::Client::new();
    let task_id = start_task(&http, &base_url).await;

    // `GET /tasks/{id}:subscribe` (spec Section 3.1.6 / 11.3.2) is the only
    // literal HTTP binding `SubscribeToTask` has; `POST` (covered above) is
    // this crate's own pre-existing, non-spec-literal addition kept for
    // backward compatibility. Both must work identically.
    let resp = http
        .get(format!("{base_url}/tasks/{task_id}:subscribe"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tasks/{id}:subscribe");
    assert_eq!(resp.status(), 200);
    let mut events = resp.bytes_stream().eventsource();

    // Fresh subscribe (no Last-Event-ID) MUST begin with a `Task` snapshot
    // (spec Section 3.1.6).
    let lead = events.next().await.expect("lead event").expect("sse event");
    let lead_value: serde_json::Value = serde_json::from_str(&lead.data).unwrap();
    assert_eq!(lead_value["task"]["id"], task_id);

    let first = events.next().await.expect("first event").expect("sse event");
    let first_value: serde_json::Value = serde_json::from_str(&first.data).unwrap();
    assert_eq!(
        first_value["statusUpdate"]["status"]["state"],
        "TASK_STATE_WORKING"
    );

    advance.notify_one();
    advance.notify_one();
    let mut saw_completed = false;
    while let Some(event) = events.next().await {
        let value: serde_json::Value = serde_json::from_str(&event.expect("sse event").data).unwrap();
        if value["statusUpdate"]["status"]["state"] == "TASK_STATE_COMPLETED" {
            saw_completed = true;
            break;
        }
    }
    assert!(saw_completed, "expected the completion event");
}

#[tokio::test]
async fn rest_get_on_a_plain_task_id_is_unaffected_by_the_subscribe_dispatch() {
    let (base_url, _advance) = spawn_stepped_server().await;
    let http = reqwest::Client::new();
    let task_id = start_task(&http, &base_url).await;

    // A plain `GET /tasks/{id}` (no `:subscribe` suffix) must still behave
    // as an ordinary `GetTask`, not get swept into the subscribe dispatch.
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

#[tokio::test]
async fn json_rpc_subscribe_sets_a_replayable_sse_event_id() {
    let (base_url, advance) = spawn_stepped_server().await;
    let http = reqwest::Client::new();

    let send_resp: serde_json::Value = http
        .post(format!("{base_url}/"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "SendMessage",
            "params": {
                "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]},
                "configuration": {"returnImmediately": true}
            }
        }))
        .send()
        .await
        .expect("POST /")
        .json()
        .await
        .expect("response body");
    let task_id = send_resp["result"]["task"]["id"]
        .as_str()
        .expect("task id")
        .to_string();

    let resp = http
        .post(format!("{base_url}/"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "SubscribeToTask",
            "params": {"id": task_id}
        }))
        .send()
        .await
        .expect("POST / (subscribe)");
    let mut events = resp.bytes_stream().eventsource();
    let first = events.next().await.expect("first event").expect("sse event");
    let seq: u64 = first.id.parse().expect("numeric SSE id");
    assert!(seq > 0);

    advance.notify_one();
    advance.notify_one();
}

/// An agent that goes `Working` -> waits for `advance` -> `AuthRequired`
/// (interrupted, not terminal, but closes the stream and tears down the
/// bus - the exact "idle but not terminal" case `subscribe_to_task`'s
/// no-live-bus branch handles).
struct InterruptedAgent {
    advance: Arc<Notify>,
}

#[async_trait]
impl AgentExecutor for InterruptedAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        self.advance.notified().await;
        events.status_with_message(
            TaskState::AuthRequired,
            Some(Message::agent_text("please authenticate")),
        );
        Ok(())
    }
}

#[tokio::test]
async fn subscribe_to_an_idle_interrupted_task_replays_the_buffer_then_a_snapshot() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    let advance = Arc::new(Notify::new());

    let card = AgentCard::new(
        "Interrupted Task Replay Test Agent",
        "An A2A agent used for rusty_a2a's idle-subscribe replay test.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    )
    .with_streaming(true);
    let server = AgentServer::new(
        card,
        Arc::new(InterruptedAgent {
            advance: advance.clone(),
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let http = reqwest::Client::new();
    let task_id = start_task(&http, &base_url).await;

    // Connect, read the `Working` event, capture its id, disconnect.
    let first_resp = http
        .post(format!("{base_url}/tasks/{task_id}:subscribe"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("POST /tasks/{id}:subscribe");
    let mut first_events = first_resp.bytes_stream().eventsource();
    let first = first_events
        .next()
        .await
        .expect("first event")
        .expect("sse event");
    let first_id: u64 = first.id.parse().expect("numeric SSE id");
    drop(first_events);

    // Advance to AuthRequired while disconnected; this closes the pump
    // and tears down the bus, so the task becomes "idle but not terminal".
    advance.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Reconnect: no live bus exists anymore, so this exercises the
    // no-bus branch of `subscribe_to_task` - replay first, then a final
    // current-state snapshot.
    let second_resp = http
        .post(format!("{base_url}/tasks/{task_id}:subscribe"))
        .header("A2A-Version", "1.0")
        .header("Last-Event-ID", first_id.to_string())
        .send()
        .await
        .expect("POST /tasks/{id}:subscribe (reconnect)");
    let mut second_events = second_resp.bytes_stream().eventsource();

    let replayed = second_events
        .next()
        .await
        .expect("replayed event")
        .expect("sse event");
    let replayed_value: serde_json::Value = serde_json::from_str(&replayed.data).unwrap();
    assert_eq!(
        replayed_value["statusUpdate"]["status"]["state"],
        "TASK_STATE_AUTH_REQUIRED"
    );

    let snapshot = second_events
        .next()
        .await
        .expect("snapshot event")
        .expect("sse event");
    let snapshot_value: serde_json::Value = serde_json::from_str(&snapshot.data).unwrap();
    assert_eq!(snapshot_value["task"]["id"], task_id);
    assert_eq!(
        snapshot_value["task"]["status"]["state"],
        "TASK_STATE_AUTH_REQUIRED"
    );

    assert!(second_events.next().await.is_none());
}
