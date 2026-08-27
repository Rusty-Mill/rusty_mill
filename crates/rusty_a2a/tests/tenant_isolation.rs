//! Covers `TaskStore` tenant isolation (spec Section 4.2): a task or push
//! notification config created under one tenant must be invisible to a
//! request naming a different tenant - or none at all - and the reverse
//! must hold too. Also confirms the "no tenant" namespace is a single,
//! consistent shared namespace (so an agent that never uses multi-tenancy
//! sees exactly its old single-tenant behavior).

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
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(
            TaskState::Completed,
            Some(Message::agent_text(format!("you said: {}", ctx.message.text()))),
        );
        Ok(())
    }
}

async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "Tenant Isolation Test Agent",
        "An A2A agent used for rusty_a2a's tenant isolation tests.",
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

#[tokio::test]
async fn task_created_under_one_tenant_is_invisible_to_another() {
    let base_url = spawn_test_server().await;
    let tenant_a = A2aClient::new(format!("{base_url}/")).with_tenant("tenant-a");
    let tenant_b = A2aClient::new(format!("{base_url}/")).with_tenant("tenant-b");

    let result = tenant_a
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    // Tenant A can fetch its own task.
    let fetched = tenant_a
        .get_task(&task_id, None)
        .await
        .expect("get_task (tenant-a)");
    assert_eq!(fetched.id, task_id);

    // Tenant B gets TaskNotFound, not the task - a wrong tenant must look
    // exactly like a wrong id, never leak that the id exists elsewhere.
    let err = tenant_b.get_task(&task_id, None).await.unwrap_err();
    match err {
        ClientError::Protocol(A2aError::TaskNotFound(_)) => {}
        other => panic!("expected TaskNotFound, got {other:?}"),
    }

    // Tenant B's cancel/subscribe on the same id must fail the same way.
    let err = tenant_b.cancel_task(&task_id).await.unwrap_err();
    match err {
        ClientError::Protocol(A2aError::TaskNotFound(_)) => {}
        other => panic!("expected TaskNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn list_tasks_only_returns_the_caller_tenants_own_tasks() {
    let base_url = spawn_test_server().await;
    let tenant_a = A2aClient::new(format!("{base_url}/")).with_tenant("tenant-a");
    let tenant_b = A2aClient::new(format!("{base_url}/")).with_tenant("tenant-b");

    let result = tenant_a
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let a_tasks = tenant_a
        .list_tasks(Default::default())
        .await
        .expect("list_tasks (tenant-a)");
    assert!(a_tasks.tasks.iter().any(|t| t.id == task_id));

    let b_tasks = tenant_b
        .list_tasks(Default::default())
        .await
        .expect("list_tasks (tenant-b)");
    assert!(!b_tasks.tasks.iter().any(|t| t.id == task_id));
}

#[tokio::test]
async fn push_notification_config_is_tenant_scoped() {
    let base_url = spawn_test_server().await;
    let tenant_a = A2aClient::new(format!("{base_url}/")).with_tenant("tenant-a");
    let tenant_b = A2aClient::new(format!("{base_url}/")).with_tenant("tenant-b");

    let result = tenant_a
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let mut config = TaskPushNotificationConfig::new("https://example.com/webhook");
    config.task_id = Some(task_id.clone());
    let created = tenant_a
        .create_push_notification_config(config)
        .await
        .expect("create_push_notification_config (tenant-a)");
    let config_id = created.id.clone().expect("server-assigned id");

    // Tenant A can read its own config back.
    let fetched = tenant_a
        .get_push_notification_config(&task_id, &config_id)
        .await
        .expect("get_push_notification_config (tenant-a)");
    assert_eq!(fetched.url, "https://example.com/webhook");

    // Tenant B can't even see the task, so every push-config operation on
    // it fails the same way tenant B's own typos would.
    let err = tenant_b
        .get_push_notification_config(&task_id, &config_id)
        .await
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::TaskNotFound(_)) => {}
        other => panic!("expected TaskNotFound, got {other:?}"),
    }

    let err = tenant_b
        .create_push_notification_config(TaskPushNotificationConfig {
            task_id: Some(task_id.clone()),
            ..TaskPushNotificationConfig::new("https://example.com/other-webhook")
        })
        .await
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::TaskNotFound(_)) => {}
        other => panic!("expected TaskNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn omitting_tenant_is_a_single_shared_namespace() {
    let base_url = spawn_test_server().await;
    // Neither client sets a tenant - this must behave exactly like it did
    // before tenant isolation existed: one shared namespace.
    let client_1 = A2aClient::new(format!("{base_url}/"));
    let client_2 = A2aClient::new(format!("{base_url}/"));

    let result = client_1
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let fetched = client_2
        .get_task(&task_id, None)
        .await
        .expect("get_task from a different (tenant-less) client");
    assert_eq!(fetched.id, task_id);
}

#[tokio::test]
async fn rest_binding_honors_tenant_via_query_string() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let send_resp: serde_json::Value = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]},
            "tenant": "tenant-a"
        }))
        .send()
        .await
        .expect("POST /message:send")
        .json()
        .await
        .expect("response body");
    let task_id = send_resp["task"]["id"].as_str().expect("task id").to_string();

    // Same tenant via query string: found.
    let same_tenant = http
        .get(format!("{base_url}/tasks/{task_id}?tenant=tenant-a"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tasks/{id}?tenant=tenant-a");
    assert_eq!(same_tenant.status(), 200);

    // No tenant at all: not found - REST's query-string tenant isn't
    // optional-in-practice once a task was created with one.
    let no_tenant = http
        .get(format!("{base_url}/tasks/{task_id}"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tasks/{id}");
    assert_eq!(no_tenant.status(), 404);

    // Different tenant: not found.
    let other_tenant = http
        .get(format!("{base_url}/tasks/{task_id}?tenant=tenant-b"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tasks/{id}?tenant=tenant-b");
    assert_eq!(other_tenant.status(), 404);
}
