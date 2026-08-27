//! Covers `AuthVerifier::authorize_task` (spec Section 13.1): "Implementations
//! MUST ensure appropriate scope limitation based on the authenticated
//! caller's authorization boundaries... even when `contextId` or other filter
//! parameters are not specified." Two callers authenticated against the same
//! `AuthVerifier` (and so sharing one `tenant` namespace) must not be able to
//! read, cancel, continue, or manage push-notification configs for each
//! other's tasks - and `ListTasks` must silently omit tasks the caller isn't
//! authorized to see, rather than either failing or leaking them.
//!
//! Ownership here is tracked in a side map the *test* populates after each
//! task is created (`OwnerVerifier::owners`), standing in for whatever an
//! application's own authorization system would really use (a database, a
//! claim on the credential, ...) - this crate has no opinion on what
//! "authorized" means, only a hook for the application to decide.

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
    TaskPushNotificationConfig, TaskState,
};

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

const ALICE_TOKEN: &str = "alice-token";
const BOB_TOKEN: &str = "bob-token";

/// Accepts either of two bearer tokens, resolving to a principal ("alice"/
/// "bob"), and authorizes a task only for the principal recorded as its
/// owner in `owners` - populated by the test itself right after each task
/// is created, since nothing about task creation here otherwise ties a
/// task to a caller identity (that's exactly the gap this hook closes).
#[derive(Clone, Default)]
struct OwnerVerifier {
    owners: Arc<Mutex<HashMap<String, String>>>,
}

impl OwnerVerifier {
    fn record_owner(&self, task_id: impl Into<String>, principal: impl Into<String>) {
        self.owners
            .lock()
            .unwrap()
            .insert(task_id.into(), principal.into());
    }
}

#[async_trait]
impl AuthVerifier for OwnerVerifier {
    async fn verify(
        &self,
        _requirement: &SecurityRequirement,
        credentials: &Credentials,
    ) -> Result<AuthContext> {
        match credentials.0.get("bearer").map(String::as_str) {
            Some(ALICE_TOKEN) => Ok(AuthContext::new("alice")),
            Some(BOB_TOKEN) => Ok(AuthContext::new("bob")),
            _ => Err(A2aError::Unauthenticated("invalid bearer token".to_string())),
        }
    }

    async fn authorize_task(
        &self,
        context: &AuthContext,
        _tenant: Option<&str>,
        task: &rusty_a2a::types::Task,
    ) -> Result<()> {
        let owners = self.owners.lock().unwrap();
        match owners.get(&task.id) {
            Some(owner) if Some(owner.as_str()) == context.principal.as_deref() => Ok(()),
            _ => Err(A2aError::PermissionDenied(format!(
                "task {} is not owned by {:?}",
                task.id, context.principal
            ))),
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

async fn spawn_test_server() -> (String, OwnerVerifier) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut card = AgentCard::new(
        "Task Authorization Scoping Test Agent",
        "An A2A agent used for rusty_a2a's per-principal task authorization tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    )
    .with_push_notifications(true);
    card.security_schemes
        .insert("bearer".to_string(), bearer_scheme());
    card.security_requirements = vec![SecurityRequirement {
        schemes: HashMap::from([(
            "bearer".to_string(),
            rusty_a2a::types::StringList { list: Vec::new() },
        )]),
    }];

    let verifier = OwnerVerifier::default();
    let server = AgentServer::new(card, Arc::new(EchoAgent)).with_auth_verifier(Arc::new(verifier.clone()));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (base_url, verifier)
}

fn expect_permission_denied(err: ClientError) {
    match err {
        ClientError::Protocol(A2aError::PermissionDenied(_)) => {}
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

async fn create_task(client: &A2aClient, verifier: &OwnerVerifier, owner: &str) -> String {
    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();
    verifier.record_owner(&task_id, owner);
    task_id
}

#[tokio::test]
async fn get_task_is_scoped_to_the_authorized_caller() {
    let (base_url, verifier) = spawn_test_server().await;
    let alice = A2aClient::new(format!("{base_url}/")).with_bearer_token(ALICE_TOKEN);
    let bob = A2aClient::new(format!("{base_url}/")).with_bearer_token(BOB_TOKEN);

    let alice_task = create_task(&alice, &verifier, "alice").await;
    let bob_task = create_task(&bob, &verifier, "bob").await;

    alice
        .get_task(&alice_task, None)
        .await
        .expect("alice can read her own task");
    bob.get_task(&bob_task, None)
        .await
        .expect("bob can read his own task");

    expect_permission_denied(alice.get_task(&bob_task, None).await.unwrap_err());
    expect_permission_denied(bob.get_task(&alice_task, None).await.unwrap_err());
}

#[tokio::test]
async fn cancel_task_is_scoped_to_the_authorized_caller() {
    let (base_url, verifier) = spawn_test_server().await;
    let alice = A2aClient::new(format!("{base_url}/")).with_bearer_token(ALICE_TOKEN);
    let bob = A2aClient::new(format!("{base_url}/")).with_bearer_token(BOB_TOKEN);

    let bob_task = create_task(&bob, &verifier, "bob").await;

    // `bob_task` is already `Completed` (EchoAgent finishes immediately),
    // so a legitimate cancel would fail with `TaskNotCancelable` anyway -
    // what this asserts is that alice is rejected *before* that check
    // ever runs, with `PermissionDenied` specifically.
    expect_permission_denied(alice.cancel_task(&bob_task).await.unwrap_err());
}

#[tokio::test]
async fn send_message_continuation_is_scoped_to_the_authorized_caller() {
    let (base_url, verifier) = spawn_test_server().await;
    let alice = A2aClient::new(format!("{base_url}/")).with_bearer_token(ALICE_TOKEN);
    let bob = A2aClient::new(format!("{base_url}/")).with_bearer_token(BOB_TOKEN);

    let bob_task = create_task(&bob, &verifier, "bob").await;

    let err = alice
        .send_message(Message::user_text("hi").with_task_id(bob_task), None)
        .await
        .unwrap_err();
    expect_permission_denied(err);
}

#[tokio::test]
async fn push_notification_config_crud_is_scoped_to_the_authorized_caller() {
    let (base_url, verifier) = spawn_test_server().await;
    let alice = A2aClient::new(format!("{base_url}/")).with_bearer_token(ALICE_TOKEN);
    let bob = A2aClient::new(format!("{base_url}/")).with_bearer_token(BOB_TOKEN);

    let bob_task = create_task(&bob, &verifier, "bob").await;

    let mut config = TaskPushNotificationConfig::new("https://example.com/hook");
    config.task_id = Some(bob_task.clone());
    expect_permission_denied(
        alice
            .create_push_notification_config(config.clone())
            .await
            .unwrap_err(),
    );

    // Bob can register one, but alice still can't read, list, or delete
    // it once it exists.
    let created = bob
        .create_push_notification_config(config)
        .await
        .expect("bob can register a push config on his own task");
    let config_id = created.id.clone().expect("server-assigned config id");

    expect_permission_denied(
        alice
            .get_push_notification_config(&bob_task, &config_id)
            .await
            .unwrap_err(),
    );
    expect_permission_denied(alice.list_push_notification_configs(&bob_task).await.unwrap_err());
    expect_permission_denied(
        alice
            .delete_push_notification_config(&bob_task, &config_id)
            .await
            .unwrap_err(),
    );

    // Bob himself is unaffected.
    bob.get_push_notification_config(&bob_task, &config_id)
        .await
        .expect("bob can still read his own config");
}

/// Also exercises the internal multi-page fetch `ListTasks` needs when
/// unauthorized tasks are interleaved with authorized ones: with tasks
/// alternating alice/bob/alice/bob/..., a single store page (sized to the
/// caller's requested `pageSize`) would otherwise come back under-filled
/// with only the alice-owned tasks it happened to contain.
#[tokio::test]
async fn list_tasks_only_returns_the_callers_own_tasks() {
    let (base_url, verifier) = spawn_test_server().await;
    let alice = A2aClient::new(format!("{base_url}/")).with_bearer_token(ALICE_TOKEN);
    let bob = A2aClient::new(format!("{base_url}/")).with_bearer_token(BOB_TOKEN);

    let mut alice_task_ids = Vec::new();
    for _ in 0..4 {
        alice_task_ids.push(create_task(&alice, &verifier, "alice").await);
        create_task(&bob, &verifier, "bob").await;
    }

    let listed = alice
        .list_tasks(Default::default())
        .await
        .expect("list_tasks")
        .tasks;
    let listed_ids: Vec<&str> = listed.iter().map(|t| t.id.as_str()).collect();
    for id in &alice_task_ids {
        assert!(
            listed_ids.contains(&id.as_str()),
            "expected alice's task {id} in her own list_tasks result, got {listed_ids:?}"
        );
    }
    assert_eq!(
        listed.len(),
        alice_task_ids.len(),
        "alice's list_tasks must contain only her own tasks, got {listed_ids:?}"
    );

    // Same request, small pageSize: the internal store-page loop must
    // still fill a full page of *authorized* tasks despite bob's
    // interleaved ones, not return a short page just because the first
    // raw store page it looked at was mostly bob's.
    let mut page = alice
        .list_tasks(rusty_a2a::types::ListTasksRequest {
            page_size: Some(2),
            ..Default::default()
        })
        .await
        .expect("list_tasks with a small pageSize");
    assert_eq!(page.tasks.len(), 2, "expected a full page of alice's own tasks");

    let mut seen: Vec<String> = page.tasks.iter().map(|t| t.id.clone()).collect();
    while !page.next_page_token.is_empty() {
        page = alice
            .list_tasks(rusty_a2a::types::ListTasksRequest {
                page_size: Some(2),
                page_token: Some(page.next_page_token.clone()),
                ..Default::default()
            })
            .await
            .expect("list_tasks (next page)");
        seen.extend(page.tasks.iter().map(|t| t.id.clone()));
    }
    seen.sort();
    let mut expected = alice_task_ids.clone();
    expected.sort();
    assert_eq!(
        seen, expected,
        "paging through all of alice's tasks must yield exactly her own set"
    );
}

#[tokio::test]
async fn public_agent_without_scheme_declared_still_works_unscoped() {
    // Sanity check that the default `authorize_task` (an unimplementing
    // verifier, or - as covered by every pre-existing test in this crate
    // - no verifier at all) doesn't change any prior behavior: a
    // completely public agent's tasks remain reachable by anyone, exactly
    // as before this feature existed.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    let card = AgentCard::new(
        "Public Agent",
        "No security requirements at all.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    let server = AgentServer::new(card, Arc::new(EchoAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let client = A2aClient::new(format!("{base_url}/"));
    let result = client
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();
    client
        .get_task(&task_id, None)
        .await
        .expect("get_task on a public agent");
}
