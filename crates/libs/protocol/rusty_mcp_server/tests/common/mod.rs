#![allow(dead_code, clippy::unwrap_used)]
//! The scenario both interop tests run against the `rmcp` client, whatever
//! the transport.

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientInfo, ClientRequest, ProgressNotificationParam,
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
    assert_eq!(names, ["add", "fail", "progress", "wait"], "{mode:?}");

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
