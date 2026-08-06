//! End-to-end tests for the tasks extension against `DemoServer`.
//!
//! The point of these is the fork in behaviour: the *same* tool must return a
//! task handle to a client that declared the extension and a plain result to
//! one that did not.

use std::time::Duration;

use rmcp::{
    ClientHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, ClientCapabilities, ClientInfo, GetTaskParams,
        ProtocolVersion, TaskStatus,
    },
    service::RunningService,
};

#[path = "../src/completions.rs"]
mod completions;
#[path = "../src/prompts.rs"]
mod prompts;
#[path = "../src/resources.rs"]
mod resources;
#[path = "../src/server.rs"]
mod server;
#[path = "../src/tools/mod.rs"]
mod tools;

use server::DemoServer;

/// A client that may or may not declare the tasks extension.
#[derive(Clone)]
struct TaskClient {
    tasks: bool,
}

impl ClientHandler for TaskClient {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        if self.tasks {
            info.capabilities = ClientCapabilities::builder().enable_tasks().build();
        }
        info
    }
}

async fn connect(tasks: bool) -> RunningService<rmcp::RoleClient, TaskClient> {
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    tokio::spawn(async move {
        let running = DemoServer::new()
            .serve(server_transport)
            .await
            .expect("server starts");
        let _ = running.waiting().await;
    });

    TaskClient { tasks }
        .serve(client_transport)
        .await
        .expect("client connects")
}

fn countdown(steps: u32) -> CallToolRequestParams {
    CallToolRequestParams::new("countdown").with_arguments(
        serde_json::json!({ "steps": steps })
            .as_object()
            .cloned()
            .expect("object"),
    )
}

#[tokio::test]
async fn a_task_capable_client_gets_a_handle_and_polls_it_to_completion() {
    let client = connect(true).await;

    let response = client
        .call_tool_once(countdown(3))
        .await
        .expect("call countdown");

    let CallToolResponse::Task(create) = response else {
        panic!("expected a task handle, got {response:?}");
    };
    assert_eq!(create.task.status, TaskStatus::Working);
    let task_id = create.task.task_id.clone();

    // Poll to a terminal state, honouring the server's suggested interval.
    let interval = create.task.poll_interval_ms.unwrap_or(50);
    let mut final_task = None;
    for _ in 0..100 {
        let got = client
            .get_task(GetTaskParams::new(task_id.clone()))
            .await
            .expect("tasks/get");
        if got.task.status().is_terminal() {
            final_task = Some(got);
            break;
        }
        tokio::time::sleep(Duration::from_millis(interval)).await;
    }

    let done = final_task.expect("task should reach a terminal state");
    assert_eq!(done.task.status(), TaskStatus::Completed);

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn a_client_without_the_extension_gets_a_plain_result() {
    // Same tool, same arguments — the fork is entirely in the client's
    // declared capabilities, which arrive per request under 2026-07-28.
    let client = connect(false).await;

    let response = client
        .call_tool_once(countdown(1))
        .await
        .expect("call countdown");

    let CallToolResponse::Complete(result) = response else {
        panic!("expected an inline result, got {response:?}");
    };
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    assert_eq!(text, "counted down 1 steps");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn fast_tools_stay_inline_even_for_task_capable_clients() {
    // The policy names only `countdown`. Handing back a handle for `add` would
    // cost the client a pointless extra round trip.
    let client = connect(true).await;

    let response = client
        .call_tool_once(
            CallToolRequestParams::new("add").with_arguments(
                serde_json::json!({ "a": 2, "b": 40 })
                    .as_object()
                    .cloned()
                    .expect("object"),
            ),
        )
        .await
        .expect("call add");

    let CallToolResponse::Complete(result) = response else {
        panic!("expected an inline result for a fast tool, got {response:?}");
    };
    assert_eq!(
        result
            .structured_content
            .expect("structured content")
            .get("sum")
            .and_then(|v| v.as_i64()),
        Some(42)
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn a_cancelled_task_settles_as_cancelled() {
    let client = connect(true).await;

    // Long enough that cancellation lands mid-flight.
    let response = client
        .call_tool_once(countdown(100))
        .await
        .expect("call countdown");

    let CallToolResponse::Task(create) = response else {
        panic!("expected a task handle");
    };
    let task_id = create.task.task_id.clone();

    client
        .cancel_task(rmcp::model::CancelTaskParams::new(task_id.clone()))
        .await
        .expect("tasks/cancel is acknowledged");

    let mut final_status = None;
    for _ in 0..100 {
        let got = client
            .get_task(GetTaskParams::new(task_id.clone()))
            .await
            .expect("tasks/get");
        if got.task.status().is_terminal() {
            final_status = Some(got.task.status());
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // The body cooperates by selecting on `cancelled()`, so it settles as
    // cancelled rather than running the remaining steps.
    assert_eq!(final_status, Some(TaskStatus::Cancelled));

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn the_server_advertises_the_tasks_extension() {
    let client = connect(true).await;

    let info = client.peer_info().expect("server info");
    assert!(
        info.capabilities.supports_tasks(),
        "server should advertise io.modelcontextprotocol/tasks"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn an_unknown_task_id_is_an_error() {
    let client = connect(true).await;

    assert!(
        client
            .get_task(GetTaskParams::new("no-such-task"))
            .await
            .is_err(),
        "polling an unknown task should fail rather than hang"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn tasks_list_still_reports_every_tool() {
    let client = connect(true).await;

    let tools = client.list_tools(None).await.expect("tools/list");
    let mut names: Vec<_> = tools.tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "add",
            "countdown",
            "divide",
            "drop_table",
            "slugify",
            "text_stats",
            "touch_resource"
        ]
    );

    client.cancel().await.expect("cancel");
}
