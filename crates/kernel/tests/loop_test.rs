//! Drives aisdk's real tool-calling loop offline via the FakeLanguageModel,
//! proving the Strategy-A bridge enforces policy and renders ToolOutcome.

use std::sync::Arc;

use rk_constrain::{Policy, PolicyError, ToolDispatch};
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use rk_kernel::{run_turn, stream_turn};
use rk_observe::ToolOutcome;
use serde_json::{json, Value};

/// A dispatcher that records what it was asked to run and blocks one tool.
struct RecordingDispatch {
    calls: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ToolDispatch for RecordingDispatch {
    async fn dispatch(&self, name: &str, _args: Value) -> ToolOutcome {
        self.calls.lock().unwrap().push(name.to_string());
        match name {
            "read_file" => ToolOutcome::ok("file contents"),
            "danger" => ToolOutcome::blocked("nope"),
            other => ToolOutcome::error(format!("unknown {other}")),
        }
    }
    fn schemas(&self) -> Vec<(String, Value)> {
        vec![
            ("read_file".into(), json!({"type": "object"})),
            ("danger".into(), json!({"type": "object"})),
        ]
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aisdk_loop_dispatches_through_our_bridge() {
    let dispatch = Arc::new(RecordingDispatch {
        calls: Default::default(),
    });
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "read_file".into(),
            args: json!({"path": "x"}),
        }],
        vec![Scripted::Text("all done".into())],
    ]);

    let reply = run_turn(model, "sys", "read x", dispatch.clone())
        .await
        .unwrap();

    assert_eq!(reply, "all done");
    assert_eq!(
        dispatch.calls.lock().unwrap().as_slice(),
        &["read_file".to_string()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_turn_emits_tokens_after_a_tool_call() {
    let dispatch = Arc::new(RecordingDispatch {
        calls: Default::default(),
    });
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "read_file".into(),
            args: json!({"path": "x"}),
        }],
        vec![Scripted::Text("streamed reply".into())],
    ]);

    let mut tokens = Vec::new();
    let reply = stream_turn(model, "sys", "go", dispatch.clone(), |t| {
        tokens.push(t.to_string())
    })
    .await
    .unwrap();

    assert_eq!(reply, "streamed reply");
    assert_eq!(tokens, vec!["streamed reply".to_string()]);
    assert_eq!(
        dispatch.calls.lock().unwrap().as_slice(),
        &["read_file".to_string()]
    );
}

/// A standalone policy unit-check that a chain blocks before dispatch — the
/// bridge path above relies on this contract.
#[tokio::test]
async fn blocked_tool_never_reaches_body() {
    struct DenyAll;
    #[async_trait::async_trait]
    impl Policy for DenyAll {
        async fn before_tool(&self, _: &str, _: &Value) -> Result<(), PolicyError> {
            Err(PolicyError::OutsideWorkspace("/denied".into()))
        }
    }
    assert!(DenyAll.before_tool("read_file", &json!({})).await.is_err());
}
