#![allow(dead_code, clippy::unwrap_used)]
//! The scenario both interop tests run against the `rmcp` client, whatever
//! the transport.

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientInfo, ClientRequest, GetPromptRequestParams,
    ProgressNotificationParam, ReadResourceRequestParams, ServerNotification, SubscriptionFilter,
};
use rmcp::service::{NotificationContext, PeerRequestOptions, RunningService};
use rmcp::{ClientHandler, RoleClient, ServiceError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Remembers every progress notification it is sent.
#[derive(Clone, Default)]
pub struct Recorder {
    pub progress: Arc<Mutex<Vec<f64>>>,
}

impl ClientHandler for Recorder {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::default()
    }

    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.progress.lock().unwrap().push(params.progress);
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Classic,
    Stateless,
}

pub fn args(json: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new("add").with_arguments(json.as_object().cloned().unwrap())
}

pub fn as_json<T: serde::Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).unwrap()
}

/// Wait for `cond`, up to five seconds.
pub async fn eventually(cond: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    cond()
}

/// The server under test offers `add`, `fail`, `progress` and `wait`, three
/// tools to a page. `saw_cancel` says whether `wait` has seen its
/// cancellation; pass `None` for a transport and mode where a client's
/// `notifications/cancelled` cannot reach the running call (Streamable HTTP
/// with the classic handshake: each POST is its own connection, so the
/// notification cannot be matched to the request). The cancel is then only
/// checked to be accepted.
pub async fn exercise(
    mode: Mode,
    client: RunningService<RoleClient, Recorder>,
    recorder: Recorder,
    saw_cancel: Option<&dyn Fn() -> bool>,
) {
    // Handshake: the client learned who it is talking to.
    let info = client.peer_info().expect("handshake completed");
    assert_eq!(as_json(&info.server_info)["name"], "interop", "{mode:?}");

    // Pagination: four tools at three per page, followed by the client.
    let first = client.list_tools(None).await.unwrap();
    assert_eq!(first.tools.len(), 3, "{mode:?}");
    assert!(first.next_cursor.is_some(), "{mode:?}");
    if matches!(mode, Mode::Stateless) {
        assert_eq!(first.ttl_ms, Some(0), "stateless lists carry cache hints");
    }
    let all = client.list_all_tools().await.unwrap();
    let names: Vec<&str> = all.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        names,
        ["add", "fail", "progress", "wait", "touch"],
        "{mode:?}"
    );

    // A call, a tool failure (a result), and malformed calls (errors).
    let sum = client
        .call_tool(args(serde_json::json!({"a": 20, "b": 22})))
        .await
        .unwrap();
    assert_eq!(as_json(&sum)["content"][0]["text"], "42", "{mode:?}");
    let failed = client
        .call_tool(CallToolRequestParams::new("fail"))
        .await
        .unwrap();
    assert_eq!(failed.is_error, Some(true), "{mode:?}");
    let missing = client.call_tool(args(serde_json::json!({"a": 1}))).await;
    assert!(
        matches!(&missing, Err(ServiceError::McpError(e)) if as_json(&e.code) == -32602),
        "{mode:?}: {missing:?}"
    );
    let unknown = client.call_tool(CallToolRequestParams::new("nope")).await;
    assert!(
        matches!(&unknown, Err(ServiceError::McpError(e)) if as_json(&e.code) == -32602),
        "{mode:?}: {unknown:?}"
    );

    exercise_prompts_resources_and_completion(&client, mode).await;

    if matches!(mode, Mode::Stateless) {
        exercise_listen(&client).await;
    }

    // Progress: the client attaches a token; the server reports against it.
    let done = client
        .call_tool(CallToolRequestParams::new("progress"))
        .await
        .unwrap();
    assert_eq!(as_json(&done)["content"][0]["text"], "finished", "{mode:?}");
    assert!(
        eventually(|| recorder.progress.lock().unwrap().len() >= 3).await,
        "{mode:?}"
    );
    assert_eq!(
        *recorder.progress.lock().unwrap(),
        [1.0, 2.0, 3.0],
        "{mode:?}"
    );

    // Cancellation: the client withdraws a running call; the tool notices.
    let handle = client
        .peer()
        .send_cancellable_request(
            ClientRequest::CallToolRequest(CallToolRequest::new(CallToolRequestParams::new(
                "wait",
            ))),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    handle.cancel(Some("test".into())).await.unwrap();
    if let Some(saw_cancel) = saw_cancel {
        assert!(
            eventually(saw_cancel).await,
            "{mode:?}: the tool never saw the cancellation"
        );
    }

    // And the connection still works afterwards.
    let again = client
        .call_tool(args(serde_json::json!({"a": 1, "b": 1})))
        .await
        .unwrap();
    assert_eq!(as_json(&again)["content"][0]["text"], "2", "{mode:?}");

    let _ = client.cancel().await;
}

fn object(json: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    json.as_object().cloned().unwrap()
}

/// Prompts, resources, templates and completion, as `rmcp` sees them.
async fn exercise_prompts_resources_and_completion(
    client: &RunningService<RoleClient, Recorder>,
    mode: Mode,
) {
    // Prompts: listed, fetched with arguments, required arguments enforced.
    let prompts = client.list_all_prompts().await.unwrap();
    let names: Vec<&str> = prompts.iter().map(|p| p.name.as_ref()).collect();
    assert_eq!(names, ["greet", "bye"], "{mode:?}");
    let greeting = client
        .get_prompt(
            GetPromptRequestParams::new("greet")
                .with_arguments(object(serde_json::json!({"name": "Ada"}))),
        )
        .await
        .unwrap();
    assert_eq!(
        as_json(&greeting)["messages"][0]["content"]["text"],
        "Hello, Ada!",
        "{mode:?}"
    );
    assert_eq!(
        as_json(&greeting)["messages"][0]["role"],
        "user",
        "{mode:?}"
    );
    let missing = client
        .get_prompt(GetPromptRequestParams::new("greet"))
        .await;
    assert!(
        matches!(&missing, Err(ServiceError::McpError(e)) if as_json(&e.code) == -32602),
        "{mode:?}: {missing:?}"
    );

    // Resources: four at three per page (so a second page is followed), two
    // templates, reads by exact URI and through each template.
    let resources = client.list_all_resources().await.unwrap();
    let names: Vec<&str> = resources.iter().map(|r| r.name.as_ref()).collect();
    assert_eq!(names, ["a", "b", "c", "d"], "{mode:?}");
    let templates = client.list_all_resource_templates().await.unwrap();
    let uris: Vec<&str> = templates.iter().map(|t| t.uri_template.as_ref()).collect();
    assert_eq!(uris, ["file:///{dir}/{name}", "log://{+path}"], "{mode:?}");
    let text = |uri: &'static str| async move {
        let read = client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .unwrap();
        as_json(&read)["contents"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(text("mem://c").await, "contents of c", "{mode:?}");
    assert_eq!(text("file:///src/lib.rs").await, "src:lib.rs", "{mode:?}");
    assert_eq!(text("log://a/b/c.log").await, "a/b/c.log", "{mode:?}");
    let gone = client
        .read_resource(ReadResourceRequestParams::new("mem://nope"))
        .await;
    assert!(
        matches!(&gone, Err(ServiceError::McpError(e)) if as_json(&e.code) == -32002),
        "{mode:?}: {gone:?}"
    );

    // Completion: for a prompt argument and for a template variable.
    let names = client
        .complete_prompt_simple("greet", "name", "A")
        .await
        .unwrap();
    assert_eq!(names, ["Ada", "Alan"], "{mode:?}");
    let dirs = client
        .complete_resource_simple("file:///{dir}/{name}", "dir", "d")
        .await
        .unwrap();
    assert_eq!(dirs, ["docs"], "{mode:?}");
    let unknown = client.complete_prompt_simple("nope", "name", "").await;
    assert!(
        matches!(&unknown, Err(ServiceError::McpError(e)) if as_json(&e.code) == -32602),
        "{mode:?}: {unknown:?}"
    );
}

/// `subscriptions/listen`: stateless clients only. The server acknowledges
/// what it granted, then forwards what `touch` publishes: an update to the
/// one resource followed, and a change to the resource list.
async fn exercise_listen(client: &RunningService<RoleClient, Recorder>) {
    let mut subscription = client
        .listen(
            SubscriptionFilter::builder()
                .resources_list_changed()
                .resource_subscription("mem://a")
                .resource_subscription("mem://b") // readable, but never published
                .build(),
        )
        .await
        .expect("subscriptions/listen");
    client
        .call_tool(CallToolRequestParams::new("touch"))
        .await
        .unwrap();

    let first = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("a notification should arrive")
        .expect("the subscription is live")
        .expect("not the end of the stream");
    let second = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("a second notification should arrive")
        .expect("the subscription is live")
        .expect("not the end of the stream");
    assert!(
        matches!(&first, ServerNotification::ResourceUpdatedNotification(n) if n.params.uri == "mem://a"),
        "unexpected first notification: {first:?}"
    );
    assert!(
        matches!(
            second,
            ServerNotification::ResourceListChangedNotification(_)
        ),
        "unexpected second notification: {second:?}"
    );
}
