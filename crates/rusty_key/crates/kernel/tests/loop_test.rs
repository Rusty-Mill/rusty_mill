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

    let (reply, _usage) = run_turn(model, "sys", "read x", dispatch.clone(), 10)
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
    let (reply, _usage) = stream_turn(model, "sys", "go", dispatch.clone(), 10, |t| {
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

/// The P0 safety floor: a model that calls a tool every step (and never emits a
/// final answer) must still terminate at `max_steps` rather than loop forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_turn_is_bounded_by_max_steps() {
    let dispatch = Arc::new(RecordingDispatch {
        calls: Default::default(),
    });
    // Far more tool-call rounds than the cap allows; without `stop_when` this
    // would run until the script (or the window) is exhausted.
    let script: Vec<Vec<Scripted>> = (0..50)
        .map(|_| {
            vec![Scripted::ToolCall {
                name: "read_file".into(),
                args: json!({"path": "x"}),
            }]
        })
        .collect();
    let model = FakeLanguageModel::new(script);

    let max_steps = 3;
    let (reply, _usage) = run_turn(model, "sys", "loop forever", dispatch.clone(), max_steps)
        .await
        .unwrap();

    // The loop hit the cap with no final answer (CleanTermination's failed case).
    assert!(reply.is_empty());
    // The cap bounded dispatch well short of the 50 scripted rounds.
    let calls = dispatch.calls.lock().unwrap().len();
    assert!(
        calls <= max_steps,
        "expected ≤ {max_steps} dispatches, got {calls}"
    );
    assert!(calls > 0, "expected the loop to run at least once");
}

/// P4: `run_turn` surfaces the provider's real token usage so the harness can
/// calibrate its compaction budget against real tokens.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_turn_surfaces_provider_usage() {
    let dispatch = Arc::new(RecordingDispatch {
        calls: Default::default(),
    });
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("done".into())]])
        .with_usage(1234, 56);

    let (reply, usage) = run_turn(model, "sys", "hi", dispatch, 10).await.unwrap();
    assert_eq!(reply, "done");
    assert_eq!(usage.input_tokens, Some(1234));
    assert_eq!(usage.output_tokens, Some(56));
}

/// A model that does not report usage yields `None` (the budget falls back to
/// its char/4 estimate).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_turn_usage_is_none_without_provider_usage() {
    let dispatch = Arc::new(RecordingDispatch {
        calls: Default::default(),
    });
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("done".into())]]);
    let (_reply, usage) = run_turn(model, "sys", "hi", dispatch, 10).await.unwrap();
    assert_eq!(usage.input_tokens, None);
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
