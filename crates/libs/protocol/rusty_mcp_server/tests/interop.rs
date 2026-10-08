#![allow(clippy::unwrap_used)]
//! The server against an independent client: the `rmcp` client, over a real
//! child process and real stdio, in both handshake modes. This is the
//! evidence that the server's reading of the spec matches the library the
//! other MCP crates use today.

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientInfo, ClientRequest, ProgressNotificationParam,
    ProtocolVersion, ServerResult,
};
use rmcp::service::ClientLifecycleMode;
use rmcp::service::{NotificationContext, PeerRequestOptions, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientHandler, ClientServiceExt, RoleClient, ServiceError, ServiceExt};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::process::Command;

/// Remembers every progress notification it is sent.
#[derive(Clone, Default)]
struct Recorder {
    progress: Arc<Mutex<Vec<f64>>>,
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
enum Mode {
    Classic,
    Stateless,
}

/// The example server, built on demand: `cargo test --test interop` alone
/// does not build examples, a plain `cargo test` does.
fn server_binary() -> PathBuf {
    // target/<profile>/deps/interop-<hash> -> target/<profile>/examples/interop_server
    let exe = std::env::current_exe().unwrap();
    let profile_dir = exe.parent().unwrap().parent().unwrap();
    let bin = profile_dir
        .join("examples")
        .join(format!("interop_server{}", std::env::consts::EXE_SUFFIX));
    if !bin.exists() {
        let mut build = std::process::Command::new(env!("CARGO"));
        build.args([
            "build",
            "--example",
            "interop_server",
            "-p",
            "rusty_mcp_server",
        ]);
        if profile_dir.file_name().is_some_and(|n| n == "release") {
            build.arg("--release");
        }
        let status = build.status().unwrap();
        assert!(status.success(), "building the example server failed");
    }
    assert!(bin.exists(), "{} missing after the build", bin.display());
    bin
}

fn unique_path(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("interop-{tag}-{}-{nanos}", std::process::id()))
}

async fn connect(
    mode: Mode,
    cancel_file: &PathBuf,
) -> (RunningService<RoleClient, Recorder>, Recorder) {
    let mut command = Command::new(server_binary());
    command.env("INTEROP_CANCEL_FILE", cancel_file);
    let transport = TokioChildProcess::new(command).unwrap();
    let recorder = Recorder::default();
    let client = match mode {
        Mode::Classic => recorder.clone().serve(transport).await.unwrap(),
        Mode::Stateless => recorder
            .clone()
            .serve_with_lifecycle(
                transport,
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                },
            )
            .await
            .unwrap(),
    };
    (client, recorder)
}

fn args(json: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new("add").with_arguments(json.as_object().cloned().unwrap())
}

fn as_json<T: serde::Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).unwrap()
}

async fn exercise(mode: Mode) {
    let cancel_file = unique_path("cancel");
    let (client, recorder) = connect(mode, &cancel_file).await;

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

    // A call, a tool failure (a result), and a malformed call (an error).
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
    let deadline = Instant::now() + Duration::from_secs(5);
    while recorder.progress.lock().unwrap().len() < 3 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cancel_file.exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        cancel_file.exists(),
        "{mode:?}: the tool never saw the cancellation"
    );
    let _ = std::fs::remove_file(&cancel_file);

    // And the connection still works afterwards.
    let again = client
        .call_tool(args(serde_json::json!({"a": 1, "b": 1})))
        .await
        .unwrap();
    assert_eq!(as_json(&again)["content"][0]["text"], "2", "{mode:?}");

    let _ = client.cancel().await;
    let _: Option<ServerResult> = None;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rmcp_client_works_against_the_server_with_the_initialize_handshake() {
    exercise(Mode::Classic).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rmcp_client_works_against_the_server_with_server_discover() {
    exercise(Mode::Stateless).await;
}
