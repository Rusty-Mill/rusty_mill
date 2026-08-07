//! Covers `AgentServer::with_webhook_ssrf_protection` (spec Section 13.2,
//! SHOULD): a push notification webhook URL that's a literal private/
//! loopback/link-local IP address is rejected at registration time when
//! enabled, and - critically - accepted by default (inert unless opted
//! into), since a local development or test setup delivering to its own
//! loopback webhook receiver is a completely legitimate use this crate
//! has no way to distinguish from an attacker probing the agent's own
//! internal network from the wire alone.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{A2aClient, ClientError};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{
    AgentCard, AgentInterface, Message, SendMessageConfiguration, TaskPushNotificationConfig, TaskState,
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

async fn spawn_test_server(ssrf_protection: bool) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "Webhook SSRF Protection Test Agent",
        "An A2A agent used for rusty_a2a's webhook SSRF protection tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    )
    .with_push_notifications(true);

    let mut server = AgentServer::new(card, Arc::new(EchoAgent));
    if ssrf_protection {
        server = server.with_webhook_ssrf_protection();
    }
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[tokio::test]
async fn loopback_and_private_webhook_urls_are_rejected_when_enabled() {
    let base_url = spawn_test_server(true).await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    for disallowed_url in [
        "http://127.0.0.1:9/hook", // loopback
        "http://10.0.0.5/hook",    // private (10.0.0.0/8)
        "http://192.168.1.1/hook", // private (192.168.0.0/16)
        "http://172.16.0.1/hook",  // private (172.16.0.0/12)
        "http://169.254.1.1/hook", // link-local
    ] {
        let mut config = TaskPushNotificationConfig::new(disallowed_url);
        config.task_id = Some(task_id.clone());
        let err = client.create_push_notification_config(config).await.unwrap_err();
        match err {
            ClientError::Protocol(A2aError::InvalidParams(_)) => {}
            other => panic!("expected InvalidParams rejecting {disallowed_url}, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_public_looking_webhook_url_is_still_accepted_when_enabled() {
    let base_url = spawn_test_server(true).await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    // 203.0.113.0/24 (TEST-NET-3, RFC 5737) is reserved for documentation
    // - never actually routable, but also not in the private/loopback/
    // link-local blocklist, so registration should succeed even though
    // delivery to it will simply fail later (that's a normal unreachable-
    // webhook outcome, not an SSRF rejection).
    let mut config = TaskPushNotificationConfig::new("http://203.0.113.5/hook");
    config.task_id = Some(task_id.clone());
    client
        .create_push_notification_config(config)
        .await
        .expect("a public-looking address should be accepted");
}

#[tokio::test]
async fn loopback_webhook_urls_are_accepted_by_default() {
    let base_url = spawn_test_server(false).await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let mut config = TaskPushNotificationConfig::new("http://127.0.0.1:9/hook");
    config.task_id = Some(task_id.clone());
    client
        .create_push_notification_config(config)
        .await
        .expect("SSRF protection is opt-in - a loopback URL must be accepted by default");
}

#[tokio::test]
async fn inline_task_push_notification_config_is_also_checked() {
    let base_url = spawn_test_server(true).await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let config = SendMessageConfiguration {
        task_push_notification_config: Some(TaskPushNotificationConfig::new("http://127.0.0.1:9/hook")),
        ..Default::default()
    };
    let err = client
        .send_message(Message::user_text("hello"), Some(config))
        .await
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::InvalidParams(_)) => {}
        other => panic!("expected InvalidParams, got {other:?}"),
    }
}
