//! Covers spec Section 13.2 (SHOULD): push notification delivery retries
//! with exponential backoff on failure, gives up after a bounded number
//! of attempts, and doesn't waste retries on a client-error response that
//! resending the identical request can't fix.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::State as AxumState;
use axum::http::StatusCode;
use axum::routing::post;
use rusty_a2a::client::A2aClient;
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{
    AgentCard, AgentInterface, Message, SendMessageConfiguration, TaskPushNotificationConfig, TaskState,
};

struct EchoAgent;

// Deliberately a single status-changing event: `notify_push_configs` fires
// on every status/artifact update, so an agent that goes `Working` then
// `Completed` (as most test agents in this crate do) would trigger two
// independent notify-and-retry sequences against the same webhook config,
// doubling every call count this suite asserts on.
#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "Webhook Retry Test Agent",
        "An A2A agent used for rusty_a2a's push notification retry tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    )
    .with_push_notifications(true);

    let server = AgentServer::new(card, Arc::new(EchoAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[derive(Clone)]
struct FlakyReceiver {
    call_count: Arc<AtomicUsize>,
    /// Returns the status this call should respond with, given the
    /// 1-indexed call number.
    respond: fn(usize) -> StatusCode,
}

async fn spawn_flaky_webhook_receiver(respond: fn(usize) -> StatusCode) -> (String, Arc<AtomicUsize>) {
    async fn handler(AxumState(receiver): AxumState<FlakyReceiver>) -> StatusCode {
        let n = receiver.call_count.fetch_add(1, Ordering::SeqCst) + 1;
        (receiver.respond)(n)
    }

    let call_count = Arc::new(AtomicUsize::new(0));
    let receiver = FlakyReceiver {
        call_count: call_count.clone(),
        respond,
    };
    let app = axum::Router::new()
        .route("/webhook", post(handler))
        .with_state(receiver);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (format!("http://{addr}/webhook"), call_count)
}

async fn send_with_inline_push_config(base_url: &str, webhook_url: String) {
    let (client, _) = A2aClient::discover(base_url).await.expect("discover");
    let config = SendMessageConfiguration {
        task_push_notification_config: Some(TaskPushNotificationConfig::new(webhook_url)),
        ..Default::default()
    };
    client
        .send_message(Message::user_text("hi"), Some(config))
        .await
        .expect("send_message");
}

#[tokio::test]
async fn delivery_succeeds_after_a_couple_of_retries() {
    let base_url = spawn_test_server().await;
    // Fail the first two attempts (server error, worth retrying), then
    // succeed on the third.
    let (webhook_url, call_count) = spawn_flaky_webhook_receiver(|n| {
        if n < 3 {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::OK
        }
    })
    .await;

    send_with_inline_push_config(&base_url, webhook_url).await;

    let mut observed = 0;
    for _ in 0..50 {
        observed = call_count.load(Ordering::SeqCst);
        if observed >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        observed, 3,
        "expected exactly 3 attempts (2 failures then a success)"
    );

    // Give it a bit longer to confirm it really stopped once successful,
    // rather than continuing to retry regardless of outcome.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn delivery_gives_up_after_the_max_attempts_on_persistent_server_errors() {
    let base_url = spawn_test_server().await;
    let (webhook_url, call_count) = spawn_flaky_webhook_receiver(|_| StatusCode::INTERNAL_SERVER_ERROR).await;

    send_with_inline_push_config(&base_url, webhook_url).await;

    // 3 retries with 200ms/400ms/800ms backoff = up to ~1.4s before the
    // final attempt; give it generous headroom.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let after_giving_up = call_count.load(Ordering::SeqCst);
    assert_eq!(
        after_giving_up, 4,
        "expected exactly 4 attempts (1 initial + 3 retries)"
    );

    // Confirm it really gave up rather than continuing indefinitely.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(call_count.load(Ordering::SeqCst), after_giving_up);
}

#[tokio::test]
async fn a_client_error_status_is_not_retried() {
    let base_url = spawn_test_server().await;
    let (webhook_url, call_count) = spawn_flaky_webhook_receiver(|_| StatusCode::BAD_REQUEST).await;

    send_with_inline_push_config(&base_url, webhook_url).await;

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "a 4xx response shouldn't be retried - resending the same request can't fix it"
    );
}
