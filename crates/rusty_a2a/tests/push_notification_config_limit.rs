//! Covers the per-task cap on registered push-notification configs
//! (`MAX_PUSH_CONFIGS_PER_TASK` in `rusty_a2a::server::store`):
//! `CreateTaskPushNotificationConfig` (spec Section 3.1.7) has no
//! spec-mandated bound on how many webhooks a client can register against
//! one task, and every status/artifact update on that task fans out into
//! one concurrent delivery attempt per registered config
//! (`notify_push_configs`) - without a cap, a client could register an
//! unbounded number of webhooks and turn a single task update into an
//! unbounded webhook-delivery amplifier.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{A2aClient, ClientError};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Message, TaskPushNotificationConfig, TaskState};

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "Push Notification Config Limit Test Agent",
        "An A2A agent used for rusty_a2a's push notification config cap tests.",
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

/// Registering configs for the same task past the cap must be rejected
/// with a clear validation error rather than accepted unboundedly.
#[tokio::test]
async fn registering_push_configs_past_the_cap_is_rejected() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    // MAX_PUSH_CONFIGS_PER_TASK is 10 - register exactly that many and
    // confirm each one succeeds.
    for i in 0..10 {
        let mut config = TaskPushNotificationConfig::new(format!("https://203.0.113.5/hook-{i}"));
        config.task_id = Some(task_id.clone());
        client
            .create_push_notification_config(config)
            .await
            .unwrap_or_else(|e| panic!("config {i} should be accepted (below the cap), got {e:?}"));
    }

    // The 11th registration for the same task must be rejected.
    let mut over_cap = TaskPushNotificationConfig::new("https://203.0.113.5/hook-overflow");
    over_cap.task_id = Some(task_id.clone());
    let err = client
        .create_push_notification_config(over_cap)
        .await
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::InvalidParams(_)) => {}
        other => panic!("expected InvalidParams rejecting the 11th push config, got {other:?}"),
    }

    // Confirm the store still only holds the 10 accepted configs, not 11.
    let listed = client
        .list_push_notification_configs(&task_id)
        .await
        .expect("list_push_notification_configs");
    assert_eq!(listed.configs.len(), 10);
}

/// Updating one of the already-registered configs (same server-assigned
/// `id`) must still work once the cap is reached - the cap only blocks
/// *new* registrations, not updates to existing ones.
#[tokio::test]
async fn updating_an_existing_config_at_the_cap_still_succeeds() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let mut first_id = None;
    for i in 0..10 {
        let mut config = TaskPushNotificationConfig::new(format!("https://203.0.113.5/hook-{i}"));
        config.task_id = Some(task_id.clone());
        let created = client
            .create_push_notification_config(config)
            .await
            .unwrap_or_else(|e| panic!("config {i} should be accepted (below the cap), got {e:?}"));
        if i == 0 {
            first_id = created.id.clone();
        }
    }

    let mut update = TaskPushNotificationConfig::new("https://203.0.113.5/hook-updated");
    update.task_id = Some(task_id.clone());
    update.id = first_id;
    client
        .create_push_notification_config(update)
        .await
        .expect("updating an existing config in place must not be blocked by the cap");

    let listed = client
        .list_push_notification_configs(&task_id)
        .await
        .expect("list_push_notification_configs");
    assert_eq!(listed.configs.len(), 10);
}
