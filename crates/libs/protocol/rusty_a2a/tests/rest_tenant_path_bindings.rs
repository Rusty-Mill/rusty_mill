//! Covers the REST binding's `additional_bindings` (spec Section 11.3):
//! every route registered again nested under a `/{tenant}` path prefix,
//! alongside the existing `tenant` body field / `?tenant=` query
//! parameter support already covered by `tests/tenant_isolation.rs`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Message, TaskState};

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
        "REST Tenant Path Binding Test Agent",
        "An A2A agent used for rusty_a2a's REST additional_bindings tests.",
        "0.0.0",
        AgentInterface::http_json(base_url.clone()),
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
async fn message_send_and_get_task_round_trip_via_tenant_path() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let send_resp: serde_json::Value = http
        .post(format!("{base_url}/tenant-a/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /{tenant}/message:send")
        .json()
        .await
        .expect("response body");
    let task_id = send_resp["task"]["id"].as_str().expect("task id").to_string();

    let get_resp = http
        .get(format!("{base_url}/tenant-a/tasks/{task_id}"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /{tenant}/tasks/{id}");
    assert_eq!(get_resp.status(), 200);

    // Invisible under a different tenant path, and under no tenant at all -
    // the path segment is a real routing/isolation boundary, not cosmetic.
    let other_tenant = http
        .get(format!("{base_url}/tenant-b/tasks/{task_id}"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tenant-b/tasks/{id}");
    assert_eq!(other_tenant.status(), 404);

    let no_tenant = http
        .get(format!("{base_url}/tasks/{task_id}"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tasks/{id}");
    assert_eq!(no_tenant.status(), 404);
}

#[tokio::test]
async fn tenant_path_segment_wins_over_a_conflicting_query_or_body_tenant() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    // The body says "tenant-x"; the path says "tenant-y". The path wins,
    // so the task must land under "tenant-y", not "tenant-x".
    let send_resp: serde_json::Value = http
        .post(format!("{base_url}/tenant-y/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]},
            "tenant": "tenant-x"
        }))
        .send()
        .await
        .expect("POST /{tenant}/message:send")
        .json()
        .await
        .expect("response body");
    let task_id = send_resp["task"]["id"].as_str().expect("task id").to_string();

    let found_under_y = http
        .get(format!("{base_url}/tenant-y/tasks/{task_id}"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tenant-y/tasks/{id}");
    assert_eq!(found_under_y.status(), 200);

    let not_under_x = http
        .get(format!("{base_url}/tenant-x/tasks/{task_id}"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tenant-x/tasks/{id}");
    assert_eq!(not_under_x.status(), 404);

    // Same precedence for the query-string form: `?tenant=` on a GET must
    // lose to the path segment too.
    let path_wins_over_query = http
        .get(format!("{base_url}/tenant-y/tasks/{task_id}?tenant=tenant-x"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tenant-y/tasks/{id}?tenant=tenant-x");
    assert_eq!(path_wins_over_query.status(), 200);
}

#[tokio::test]
async fn list_tasks_via_tenant_path() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let send_resp: serde_json::Value = http
        .post(format!("{base_url}/tenant-a/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /{tenant}/message:send")
        .json()
        .await
        .expect("response body");
    let task_id = send_resp["task"]["id"].as_str().expect("task id").to_string();

    let listed: serde_json::Value = http
        .get(format!("{base_url}/tenant-a/tasks"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /{tenant}/tasks")
        .json()
        .await
        .expect("response body");
    assert!(listed["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["id"] == task_id));

    let listed_other: serde_json::Value = http
        .get(format!("{base_url}/tenant-b/tasks"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tenant-b/tasks")
        .json()
        .await
        .expect("response body");
    assert!(listed_other["tasks"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn cancel_and_subscribe_action_suffixes_via_tenant_path() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let send_resp: serde_json::Value = http
        .post(format!("{base_url}/tenant-a/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /{tenant}/message:send")
        .json()
        .await
        .expect("response body");
    let task_id = send_resp["task"]["id"].as_str().expect("task id").to_string();

    // `EchoAgent` finishes synchronously, so the task is already
    // `Completed` by the time we get here - `GET .../{id}:subscribe`
    // under the tenant path must reject it the same way every other
    // binding rejects subscribing to a terminal task (spec Section
    // 3.1.6), proving the path segment reached `SubscribeToTask` at all
    // rather than 404ing or being silently ignored.
    let subscribe_resp = http
        .get(format!("{base_url}/tenant-a/tasks/{task_id}:subscribe"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /{tenant}/tasks/{id}:subscribe");
    assert_eq!(subscribe_resp.status(), 400);
    let subscribe_body: serde_json::Value = subscribe_resp.json().await.expect("response body");
    assert_eq!(
        subscribe_body["error"]["details"][0]["reason"],
        "UNSUPPORTED_OPERATION"
    );

    // The task is already `Completed` by the time we get here (`EchoAgent`
    // finishes synchronously), so `:cancel` under the tenant path must
    // fail exactly the way it would without the tenant path - proving the
    // path segment reached `CancelTask` at all rather than silently
    // 404ing or being ignored.
    let cancel_resp = http
        .post(format!("{base_url}/tenant-a/tasks/{task_id}:cancel"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("POST /{tenant}/tasks/{id}:cancel");
    assert_eq!(cancel_resp.status(), 400);
    let body: serde_json::Value = cancel_resp.json().await.expect("response body");
    assert_eq!(body["error"]["details"][0]["reason"], "TASK_NOT_CANCELABLE");
}

#[tokio::test]
async fn push_notification_config_crud_via_tenant_path() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let send_resp: serde_json::Value = http
        .post(format!("{base_url}/tenant-a/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /{tenant}/message:send")
        .json()
        .await
        .expect("response body");
    let task_id = send_resp["task"]["id"].as_str().expect("task id").to_string();

    let created: serde_json::Value = http
        .post(format!(
            "{base_url}/tenant-a/tasks/{task_id}/pushNotificationConfigs"
        ))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({"url": "https://example.com/webhook"}))
        .send()
        .await
        .expect("POST /{tenant}/.../pushNotificationConfigs")
        .json()
        .await
        .expect("response body");
    let config_id = created["id"].as_str().expect("config id").to_string();

    // Not visible under a different tenant path.
    let wrong_tenant = http
        .get(format!(
            "{base_url}/tenant-b/tasks/{task_id}/pushNotificationConfigs/{config_id}"
        ))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tenant-b/.../pushNotificationConfigs/{configId}");
    assert_eq!(wrong_tenant.status(), 404);

    let fetched = http
        .get(format!(
            "{base_url}/tenant-a/tasks/{task_id}/pushNotificationConfigs/{config_id}"
        ))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /{tenant}/.../pushNotificationConfigs/{configId}");
    assert_eq!(fetched.status(), 200);

    let listed: serde_json::Value = http
        .get(format!(
            "{base_url}/tenant-a/tasks/{task_id}/pushNotificationConfigs"
        ))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /{tenant}/.../pushNotificationConfigs")
        .json()
        .await
        .expect("response body");
    assert_eq!(listed["configs"].as_array().unwrap().len(), 1);

    let delete_resp = http
        .delete(format!(
            "{base_url}/tenant-a/tasks/{task_id}/pushNotificationConfigs/{config_id}"
        ))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("DELETE /{tenant}/.../pushNotificationConfigs/{configId}");
    assert_eq!(delete_resp.status(), 204);

    let after_delete = http
        .get(format!(
            "{base_url}/tenant-a/tasks/{task_id}/pushNotificationConfigs/{config_id}"
        ))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /{tenant}/.../pushNotificationConfigs/{configId}");
    assert_eq!(after_delete.status(), 404);
}

#[tokio::test]
async fn extended_agent_card_via_tenant_path() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    // This test agent's card never sets `capabilities.extendedAgentCard`,
    // so this is `UnsupportedOperation` rather than success - the point
    // isn't the specific error, it's that the route is reachable at all
    // under the tenant prefix (the tenant segment doesn't select anything
    // for this operation - see `get_extended_agent_card_tenant`'s doc
    // comment) and actually reached `Engine::get_extended_agent_card`
    // rather than 404ing or being silently ignored.
    let resp = http
        .get(format!("{base_url}/tenant-a/extendedAgentCard"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /{tenant}/extendedAgentCard");
    let body: serde_json::Value = resp.json().await.expect("response body");
    assert_eq!(body["error"]["details"][0]["reason"], "UNSUPPORTED_OPERATION");
}

#[tokio::test]
async fn a_tenant_named_like_a_literal_route_segment_does_not_break_routing() {
    // `/tasks/tasks` is ambiguous between `GetTask(id="tasks")` (the
    // top-level `/tasks/:id` route) and `ListTasks` under a tenant
    // literally named "tasks" (`/:tenant/tasks`). `axum`'s router
    // resolves this deterministically in favor of the more specific
    // static-first-segment route, so this must behave as `GetTask`, not
    // silently 500 or panic at router construction.
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .get(format!("{base_url}/tasks/tasks"))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .expect("GET /tasks/tasks");
    // Not a real task id, so this is a 404 either way - the point is that
    // the server responds deterministically instead of erroring out of
    // the router itself.
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.expect("response body");
    assert_eq!(body["error"]["details"][0]["reason"], "TASK_NOT_FOUND");
}
